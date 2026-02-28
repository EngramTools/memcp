//! Temporal event-time extraction from memory content.
//!
//! Parses time references in memory content ("in 2019", "when I was 6", "in the 90s")
//! into structured event_time + precision metadata for temporal queries.
//!
//! Distinct from `intelligence/query_intelligence/temporal.rs` which handles
//! search query rewriting (e.g., "last week"). This module handles storage-time
//! extraction from content being stored.

use std::sync::OnceLock;

use chrono::{DateTime, Datelike, TimeZone, Utc, Weekday};
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Precision of the extracted event time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EventTimePrecision {
    Decade,
    Year,
    Month,
    Day,
}

impl EventTimePrecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Decade => "decade",
            Self::Year => "year",
            Self::Month => "month",
            Self::Day => "day",
        }
    }
}

// ---------------------------------------------------------------------------
// Static regex patterns (compiled once via OnceLock)
// ---------------------------------------------------------------------------

/// Pattern 4: month-year — "in March 2020", "in January 1995"
/// Most specific, checked first.
static RE_MONTH_YEAR: OnceLock<Regex> = OnceLock::new();
fn re_month_year() -> &'static Regex {
    RE_MONTH_YEAR.get_or_init(|| {
        Regex::new(
            r"(?i)\bin\s+(January|February|March|April|May|June|July|August|September|October|November|December)\s+(\d{4})\b"
        ).expect("RE_MONTH_YEAR compile error")
    })
}

/// Pattern 1: absolute year — "in 2019", "in 1985"
static RE_YEAR: OnceLock<Regex> = OnceLock::new();
fn re_year() -> &'static Regex {
    RE_YEAR.get_or_init(|| {
        Regex::new(r"\bin\s+(\d{4})\b").expect("RE_YEAR compile error")
    })
}

/// Pattern 2: decade — "in the 90s", "in the 80s"
static RE_DECADE: OnceLock<Regex> = OnceLock::new();
fn re_decade() -> &'static Regex {
    RE_DECADE.get_or_init(|| {
        Regex::new(r"(?i)\bin\s+the\s+(\d{2})s\b").expect("RE_DECADE compile error")
    })
}

/// Pattern 3: relative age — "when I was 6", "when I was 25"
static RE_RELATIVE_AGE: OnceLock<Regex> = OnceLock::new();
fn re_relative_age() -> &'static Regex {
    RE_RELATIVE_AGE.get_or_init(|| {
        Regex::new(r"(?i)\bwhen\s+I\s+was\s+(\d{1,3})\b").expect("RE_RELATIVE_AGE compile error")
    })
}

/// Pattern 5: relative month — "last March", "last January"
static RE_RELATIVE_MONTH: OnceLock<Regex> = OnceLock::new();
fn re_relative_month() -> &'static Regex {
    RE_RELATIVE_MONTH.get_or_init(|| {
        Regex::new(
            r"(?i)\blast\s+(January|February|March|April|May|June|July|August|September|October|November|December)\b"
        ).expect("RE_RELATIVE_MONTH compile error")
    })
}

/// Pattern 6: relative day — "last Tuesday", "last Monday"
static RE_RELATIVE_DAY: OnceLock<Regex> = OnceLock::new();
fn re_relative_day() -> &'static Regex {
    RE_RELATIVE_DAY.get_or_init(|| {
        Regex::new(
            r"(?i)\blast\s+(Monday|Tuesday|Wednesday|Thursday|Friday|Saturday|Sunday)\b"
        ).expect("RE_RELATIVE_DAY compile error")
    })
}

// ---------------------------------------------------------------------------
// Month/weekday name helpers
// ---------------------------------------------------------------------------

fn month_number(name: &str) -> Option<u32> {
    match name.to_lowercase().as_str() {
        "january" => Some(1),
        "february" => Some(2),
        "march" => Some(3),
        "april" => Some(4),
        "may" => Some(5),
        "june" => Some(6),
        "july" => Some(7),
        "august" => Some(8),
        "september" => Some(9),
        "october" => Some(10),
        "november" => Some(11),
        "december" => Some(12),
        _ => None,
    }
}

fn weekday_from_name(name: &str) -> Option<Weekday> {
    match name.to_lowercase().as_str() {
        "monday" => Some(Weekday::Mon),
        "tuesday" => Some(Weekday::Tue),
        "wednesday" => Some(Weekday::Wed),
        "thursday" => Some(Weekday::Thu),
        "friday" => Some(Weekday::Fri),
        "saturday" => Some(Weekday::Sat),
        "sunday" => Some(Weekday::Sun),
        _ => None,
    }
}

/// Find the most recent past occurrence of `target_weekday` before `now` (exclusive).
/// Returns the start of that day (midnight UTC).
fn last_weekday(target: Weekday, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    // Days to subtract: if today is the target weekday, go back 7 days; otherwise subtract difference.
    let now_weekday = now.weekday();
    let now_num = now_weekday.num_days_from_monday(); // Mon=0..Sun=6
    let target_num = target.num_days_from_monday();

    let days_back = if now_num > target_num {
        now_num - target_num
    } else if now_num == target_num {
        7 // "last Tuesday" when today IS Tuesday means 7 days ago
    } else {
        7 - (target_num - now_num)
    };

    let date = now.date_naive() - chrono::Duration::days(days_back as i64);
    Utc.with_ymd_and_hms(date.year(), date.month(), date.day(), 0, 0, 0)
        .single()
}

/// Find the most recent past occurrence of `target_month` before `now`.
/// Returns 1st of that month, midnight UTC.
fn last_month_occurrence(target_month: u32, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let now_month = now.month();
    let year = if target_month < now_month {
        // Target month is earlier this year
        now.year()
    } else if target_month == now_month {
        // Same month — "last March" when it's March means last year
        now.year() - 1
    } else {
        // Target month is later — it hasn't happened yet this year, go to last year
        now.year() - 1
    };
    Utc.with_ymd_and_hms(year, target_month, 1, 0, 0, 0).single()
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Extract event time from memory content using regex patterns.
///
/// Returns the most specific temporal reference found, scanning patterns in
/// priority order: month-year > year > decade > relative-age > relative-month > relative-day.
///
/// # Arguments
/// - `content` — the memory content text to scan
/// - `birth_year` — user's birth year (required for relative-age patterns like "when I was 6")
/// - `now` — reference time for relative patterns ("last March", "last Tuesday")
///
/// # Returns
/// `Some((event_time, precision))` if a pattern matched, `None` otherwise.
pub fn extract_event_time(
    content: &str,
    birth_year: Option<u32>,
    now: DateTime<Utc>,
) -> Option<(DateTime<Utc>, EventTimePrecision)> {
    // Pattern 4: month-year (most specific)
    if let Some(caps) = re_month_year().captures(content) {
        let month_name = caps.get(1)?.as_str();
        let year_str = caps.get(2)?.as_str();
        if let (Some(month), Ok(year)) = (month_number(month_name), year_str.parse::<i32>()) {
            if let Some(dt) = Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0).single() {
                return Some((dt, EventTimePrecision::Month));
            }
        }
    }

    // Pattern 1: absolute year
    if let Some(caps) = re_year().captures(content) {
        let year_str = caps.get(1)?.as_str();
        if let Ok(year) = year_str.parse::<i32>() {
            if (1900..=2100).contains(&year) {
                if let Some(dt) = Utc.with_ymd_and_hms(year, 1, 1, 0, 0, 0).single() {
                    return Some((dt, EventTimePrecision::Year));
                }
            }
        }
    }

    // Pattern 2: decade ("in the 90s")
    // The regex captures the two-digit decade prefix: "90" from "the 90s", "80" from "the 80s".
    // "90" is the decade within a century: 1990s = 1990, 2090s = 2090.
    // Pick the most recent past century.
    if let Some(caps) = re_decade().captures(content) {
        let decade_str = caps.get(1)?.as_str();
        if let Ok(decade_prefix) = decade_str.parse::<i32>() {
            // decade_prefix = 90 → offset within century (90)
            // year_1900s = 1900 + decade_prefix = 1990
            // year_2000s = 2000 + decade_prefix = 2090
            let year_1900s = 1900 + decade_prefix;
            let year_2000s = 2000 + decade_prefix;
            // Use the most recent past decade (≤ current year)
            let year = if year_2000s <= now.year() {
                year_2000s
            } else {
                year_1900s
            };
            if let Some(dt) = Utc.with_ymd_and_hms(year, 1, 1, 0, 0, 0).single() {
                return Some((dt, EventTimePrecision::Decade));
            }
        }
    }

    // Pattern 3: relative age ("when I was 6") — requires birth_year
    if let Some(birth) = birth_year {
        if let Some(caps) = re_relative_age().captures(content) {
            let age_str = caps.get(1)?.as_str();
            if let Ok(age) = age_str.parse::<u32>() {
                let event_year = (birth + age) as i32;
                if let Some(dt) = Utc.with_ymd_and_hms(event_year, 1, 1, 0, 0, 0).single() {
                    return Some((dt, EventTimePrecision::Year));
                }
            }
        }
    }

    // Pattern 5: relative month ("last March")
    if let Some(caps) = re_relative_month().captures(content) {
        let month_name = caps.get(1)?.as_str();
        if let Some(month) = month_number(month_name) {
            if let Some(dt) = last_month_occurrence(month, now) {
                return Some((dt, EventTimePrecision::Month));
            }
        }
    }

    // Pattern 6: relative day ("last Tuesday")
    if let Some(caps) = re_relative_day().captures(content) {
        let day_name = caps.get(1)?.as_str();
        if let Some(weekday) = weekday_from_name(day_name) {
            if let Some(dt) = last_weekday(weekday, now) {
                return Some((dt, EventTimePrecision::Day));
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(year: i32, month: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, 0, 0, 0)
            .single()
            .unwrap()
    }

    #[test]
    fn test_absolute_year() {
        let now = utc(2026, 6, 15);
        let result = extract_event_time("I moved to London in 2019", None, now);
        assert_eq!(result, Some((utc(2019, 1, 1), EventTimePrecision::Year)));
    }

    #[test]
    fn test_decade() {
        let now = utc(2026, 6, 15);
        let result = extract_event_time("I grew up in the 90s", None, now);
        assert_eq!(result, Some((utc(1990, 1, 1), EventTimePrecision::Decade)));
    }

    #[test]
    fn test_decade_80s() {
        let now = utc(2026, 6, 15);
        let result = extract_event_time("Music was better in the 80s", None, now);
        assert_eq!(result, Some((utc(1980, 1, 1), EventTimePrecision::Decade)));
    }

    #[test]
    fn test_relative_age_with_birth_year() {
        let now = utc(2026, 6, 15);
        let result = extract_event_time("when I was 6 I learned to swim", Some(1990), now);
        assert_eq!(result, Some((utc(1996, 1, 1), EventTimePrecision::Year)));
    }

    #[test]
    fn test_relative_age_without_birth_year() {
        let now = utc(2026, 6, 15);
        let result = extract_event_time("when I was 6 I learned to swim", None, now);
        // Should not match because birth_year is None
        assert_eq!(result, None);
    }

    #[test]
    fn test_month_year() {
        let now = utc(2026, 6, 15);
        let result = extract_event_time("I started the project in March 2020", None, now);
        assert_eq!(result, Some((utc(2020, 3, 1), EventTimePrecision::Month)));
    }

    #[test]
    fn test_last_march_before_march() {
        // now is Feb 15, 2026 — "last March" should be March 2025
        let now = utc(2026, 2, 15);
        let result = extract_event_time("I saw them last March", None, now);
        assert_eq!(result, Some((utc(2025, 3, 1), EventTimePrecision::Month)));
    }

    #[test]
    fn test_last_march_after_march() {
        // now is June 15, 2026 — "last March" should be March 2026
        let now = utc(2026, 6, 15);
        let result = extract_event_time("I saw them last March", None, now);
        assert_eq!(result, Some((utc(2026, 3, 1), EventTimePrecision::Month)));
    }

    #[test]
    fn test_no_temporal_reference() {
        let now = utc(2026, 6, 15);
        let result = extract_event_time("I like pizza and coffee", None, now);
        assert_eq!(result, None);
    }

    #[test]
    fn test_multiple_references_first_wins() {
        // month-year wins over plain year since it's checked first
        let now = utc(2026, 6, 15);
        let result = extract_event_time("I started in March 2020 and finished in 2021", None, now);
        // month-year pattern matches "in March 2020" first
        assert_eq!(result, Some((utc(2020, 3, 1), EventTimePrecision::Month)));
    }

    #[test]
    fn test_year_out_of_range() {
        let now = utc(2026, 6, 15);
        // year 1800 is before valid range 1900-2100
        let result = extract_event_time("in 1800 people used horses", None, now);
        assert_eq!(result, None);
    }

    #[test]
    fn test_last_tuesday() {
        // now is Wednesday 2026-06-17 (Wednesday)
        // "last Tuesday" should be 2026-06-16
        let now = utc(2026, 6, 17); // Wednesday
        let result = extract_event_time("I met them last Tuesday", None, now);
        assert_eq!(result, Some((utc(2026, 6, 16), EventTimePrecision::Day)));
    }

    #[test]
    fn test_last_tuesday_when_today_is_tuesday() {
        // now is Tuesday 2026-06-16 — "last Tuesday" should be 7 days ago: 2026-06-09
        let now = utc(2026, 6, 16); // Tuesday
        let result = extract_event_time("I met them last Tuesday", None, now);
        assert_eq!(result, Some((utc(2026, 6, 9), EventTimePrecision::Day)));
    }

    #[test]
    fn test_case_insensitive_month() {
        let now = utc(2026, 6, 15);
        let result = extract_event_time("in march 2020 we started", None, now);
        assert_eq!(result, Some((utc(2020, 3, 1), EventTimePrecision::Month)));
    }

    #[test]
    fn test_case_insensitive_when_i_was() {
        let now = utc(2026, 6, 15);
        let result = extract_event_time("When I Was 10 years old", Some(1990), now);
        assert_eq!(result, Some((utc(2000, 1, 1), EventTimePrecision::Year)));
    }
}

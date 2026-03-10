//! Temporal event-time extraction from memory content.
//!
//! Parses time references in memory content ("in 2019", "when I was 6", "in the 90s")
//! into structured event_time + precision metadata for temporal queries.
//!
//! Distinct from `intelligence/query_intelligence/temporal.rs` which handles
//! search query rewriting (e.g., "last week"). This module handles storage-time
//! extraction from content being stored.

use std::sync::{Arc, OnceLock};

use chrono::{DateTime, Datelike, TimeZone, Utc, Weekday};
use metrics;
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
    RE_YEAR.get_or_init(|| Regex::new(r"\bin\s+(\d{4})\b").expect("RE_YEAR compile error"))
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
        Regex::new(r"(?i)\blast\s+(Monday|Tuesday|Wednesday|Thursday|Friday|Saturday|Sunday)\b")
            .expect("RE_RELATIVE_DAY compile error")
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
    Utc.with_ymd_and_hms(year, target_month, 1, 0, 0, 0)
        .single()
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
// LLM temporal extraction (async)
// ---------------------------------------------------------------------------

/// LLM-based temporal extraction for subtle references regex cannot catch.
///
/// Sends content to an LLM with a structured prompt requesting temporal references.
/// Returns `(event_time, precision)` or `None` if no temporal reference is found or
/// the call times out / fails (fail-open).
///
/// # Arguments
/// - `content`    — memory content text to analyze
/// - `birth_year` — user birth year for age-relative resolution
/// - `provider`   — "ollama" or "openai"
/// - `model`      — model name (e.g. "llama3.2:3b" or "gpt-4o-mini")
/// - `api_key`    — OpenAI API key (ignored for Ollama)
/// - `base_url`   — base URL override (e.g. "<http://localhost:11434>")
/// - `now`        — reference time for relative expressions
pub async fn extract_event_time_llm(
    content: &str,
    birth_year: Option<u32>,
    provider: &str,
    model: &str,
    api_key: Option<&str>,
    base_url: Option<&str>,
    now: DateTime<Utc>,
) -> Option<(DateTime<Utc>, EventTimePrecision)> {
    let birth_context = birth_year
        .map(|y| format!("The person was born in {}. ", y))
        .unwrap_or_default();

    let prompt = format!(
        "{}Analyze this text and extract any temporal reference to when the described event happened. \
         Return ONLY a JSON object like {{\"year\": 2019, \"month\": 3, \"day\": null}} \
         where month and day are null if unknown, OR the string \"none\" if there is no temporal reference. \
         Do not explain — only the JSON or \"none\".\nText: {}",
        birth_context, content
    );

    let raw_response = match provider {
        "openai" => {
            let key = api_key.unwrap_or("");
            let base = base_url.unwrap_or("https://api.openai.com/v1");
            call_openai_temporal(base, key, model, &prompt).await
        }
        _ => {
            // Default to Ollama
            let base = base_url.unwrap_or("http://localhost:11434");
            call_ollama_temporal(base, model, &prompt).await
        }
    };

    let raw = match raw_response {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(error = %e, "LLM temporal extraction failed (fail-open)");
            return None;
        }
    };

    let trimmed = raw.trim();
    if trimmed == "none" || trimmed.is_empty() {
        return None;
    }

    // Parse JSON response: {"year": YYYY, "month": MM_or_null, "day": DD_or_null}
    let v: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => {
            tracing::debug!(raw = %trimmed, "LLM temporal response not valid JSON");
            return None;
        }
    };

    let year = v.get("year").and_then(|y| y.as_i64())? as i32;
    if !(1900..=2100).contains(&year) {
        return None;
    }

    let month = v.get("month").and_then(|m| m.as_i64()).unwrap_or(1).max(1) as u32;
    let day = v.get("day").and_then(|d| d.as_i64()).unwrap_or(1).max(1) as u32;

    let precision = if v.get("month").and_then(|m| m.as_i64()).is_some() {
        if v.get("day").and_then(|d| d.as_i64()).is_some() {
            EventTimePrecision::Day
        } else {
            EventTimePrecision::Month
        }
    } else {
        EventTimePrecision::Year
    };

    let _ = now; // now is available for future relative-reference handling
    Utc.with_ymd_and_hms(year, month, day, 0, 0, 0)
        .single()
        .map(|dt| (dt, precision))
}

/// Call Ollama /api/chat endpoint for temporal extraction.
async fn call_ollama_temporal(base_url: &str, model: &str, prompt: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "stream": false,
        "options": {"temperature": 0.0}
    });
    let url = format!("{}/api/chat", base_url);

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        client.post(&url).json(&body).send(),
    )
    .await
    .map_err(|_| "timeout".to_string())?
    .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("Ollama returned {}", resp.status()));
    }

    let parsed: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    parsed
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "no content in Ollama response".to_string())
}

/// Call OpenAI /chat/completions endpoint for temporal extraction.
async fn call_openai_temporal(
    base_url: &str,
    api_key: &str,
    model: &str,
    prompt: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": 64,
        "temperature": 0.0
    });
    let url = format!("{}/chat/completions", base_url);

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&body)
            .send(),
    )
    .await
    .map_err(|_| "timeout".to_string())?
    .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("OpenAI returned {}", resp.status()));
    }

    let parsed: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    parsed
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "no content in OpenAI response".to_string())
}

// ---------------------------------------------------------------------------
// Temporal LLM background worker
// ---------------------------------------------------------------------------

/// Background worker that applies LLM temporal extraction to memories missing event_time.
///
/// Polls for memories with no event_time (already fully extracted otherwise),
/// calls `extract_event_time_llm` for each, and updates `event_time` + `event_time_precision`.
/// Gated on `config.temporal.llm_enabled` — returns immediately when disabled.
///
/// Poll interval: 60 seconds (low-priority background work).
pub async fn run_temporal_worker(
    store: Arc<crate::store::postgres::PostgresMemoryStore>,
    config: &crate::config::TemporalConfig,
    birth_year: Option<u32>,
    mut shutdown: tokio::sync::broadcast::Receiver<()>,
) {
    if !config.llm_enabled {
        tracing::debug!("Temporal LLM worker disabled via config");
        return;
    }

    tracing::info!(
        provider = %config.provider,
        "Temporal LLM background worker started"
    );

    let provider = config.provider.clone();
    let model = match provider.as_str() {
        "openai" => config.openai_model.clone(),
        _ => config.ollama_model.clone(),
    };
    let api_key = config.openai_api_key.clone();
    let base_url = config.openai_base_url.clone();

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));

    loop {
        tokio::select! {
            _ = shutdown.recv() => {
                tracing::debug!("Temporal LLM worker received shutdown signal");
                return;
            }
            _ = interval.tick() => {
                // Fetch memories missing event_time (up to 10 per pass)
                let candidates = match sqlx::query_as::<_, (String, String)>(
                    "SELECT id, content FROM memories \
                     WHERE event_time IS NULL AND deleted_at IS NULL \
                     AND extraction_status = 'complete' \
                     ORDER BY created_at DESC LIMIT 10"
                )
                .fetch_all(store.pool())
                .await
                {
                    Ok(rows) => rows,
                    Err(e) => {
                        tracing::warn!(error = %e, "Temporal LLM worker: failed to fetch candidates");
                        continue;
                    }
                };

                if candidates.is_empty() {
                    continue;
                }

                tracing::debug!(count = candidates.len(), "Temporal LLM worker: processing candidates");

                let now = Utc::now();
                for (memory_id, content) in &candidates {
                    let result = extract_event_time_llm(
                        content,
                        birth_year,
                        &provider,
                        &model,
                        api_key.as_deref(),
                        base_url.as_deref(),
                        now,
                    )
                    .await;

                    if let Some((event_time, precision)) = result {
                        if let Err(e) = store.update_event_time(memory_id, event_time, precision.as_str()).await {
                            tracing::warn!(
                                error = %e,
                                memory_id = %memory_id,
                                "Temporal LLM worker: failed to update event_time"
                            );
                        } else {
                            tracing::debug!(
                                memory_id = %memory_id,
                                event_time = %event_time,
                                precision = %precision.as_str(),
                                "Temporal LLM worker: updated event_time"
                            );
                            metrics::counter!("memcp_temporal_extractions_total").increment(1);
                        }
                    }
                }
            }
        }
    }
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

/// Regex-based content filter using RegexSet for efficient multi-pattern matching.
///
/// All patterns are compiled once at startup. Invalid patterns cause a startup error.
/// Uses RegexSet for single-pass matching against all patterns.

use regex::RegexSet;
use crate::errors::MemcpError;

/// Fast regex-based content filter.
///
/// Compiled once from config patterns. Thread-safe and immutable after construction.
pub struct RegexFilter {
    patterns: RegexSet,
    /// Original pattern strings for diagnostic/logging purposes
    pattern_strings: Vec<String>,
}

impl RegexFilter {
    /// Create a new RegexFilter from pattern strings.
    ///
    /// Validates and compiles all patterns at once. Returns error if any pattern is invalid.
    pub fn new(patterns: &[String]) -> Result<Self, MemcpError> {
        let set = RegexSet::new(patterns).map_err(|e| {
            MemcpError::Config(format!("Invalid content filter regex pattern: {}", e))
        })?;
        tracing::info!(pattern_count = patterns.len(), "Content filter: regex patterns compiled");
        Ok(RegexFilter {
            patterns: set,
            pattern_strings: patterns.to_vec(),
        })
    }

    /// Check if content matches any exclusion pattern.
    ///
    /// Returns the first matched pattern string for logging, or None if no match.
    pub fn matches(&self, content: &str) -> Option<String> {
        let matches: Vec<usize> = self.patterns.matches(content).into_iter().collect();
        if matches.is_empty() {
            None
        } else {
            Some(self.pattern_strings[matches[0]].clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_regex_filter_matches() {
        let patterns = vec![
            r"(?i)\b(password|secret|api.key)\b.*=.*".to_string(),
            r"(?i)\b(ssn|social.security)\b".to_string(),
        ];
        let filter = RegexFilter::new(&patterns).unwrap();

        assert!(filter.matches("password = hunter2").is_some());
        assert!(filter.matches("API_KEY = abc123").is_some());
        assert!(filter.matches("my social security number").is_some());
        assert!(filter.matches("the weather is nice").is_none());
    }

    #[test]
    fn test_regex_filter_invalid_pattern() {
        let patterns = vec!["[invalid".to_string()];
        assert!(RegexFilter::new(&patterns).is_err());
    }

    #[test]
    fn test_regex_filter_empty_patterns() {
        let filter = RegexFilter::new(&[]).unwrap();
        assert!(filter.matches("anything").is_none());
    }
}

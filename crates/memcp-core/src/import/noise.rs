//! Noise filter — drops low-signal chunks before dedup and batch insert.
//!
//! Two-layer filter:
//! 1. Minimum character length (default 50)
//! 2. Pattern matching (case-insensitive contains) against configurable patterns

/// Default minimum content length. Chunks below this are considered noise.
pub const DEFAULT_MIN_CHARS: usize = 50;

/// Hardcoded noise patterns for OpenClaw session logs.
/// These are the most common system-generated noise entries in OpenClaw data.
pub const OPENCLAW_NOISE_PATTERNS: &[&str] = &[
    "HEARTBEAT_OK",
    "Token Monitor Report",
    "Switchboard - Cross-Subagent",
    "FailoverError: LLM request timed out",
    "Exec failed",
    "Exec completed",
    "compinit: initialization aborted",
];

/// Rule-based noise filter. Checks minimum length and pattern matches.
pub struct NoiseFilter {
    /// Combined list of noise patterns (source defaults + user config + CLI skip patterns).
    patterns: Vec<String>,
    /// Minimum character count — content shorter than this is dropped.
    pub min_chars: usize,
}

impl NoiseFilter {
    /// Create a filter with only user-supplied patterns (no source-specific defaults).
    pub fn new(extra_patterns: &[String]) -> Self {
        Self {
            patterns: extra_patterns.to_vec(),
            min_chars: DEFAULT_MIN_CHARS,
        }
    }

    /// Create a filter combining source-specific patterns and user-supplied patterns.
    pub fn new_with_source_patterns(extra_patterns: &[String], source_patterns: &[String]) -> Self {
        let mut patterns: Vec<String> = source_patterns.to_vec();
        patterns.extend(extra_patterns.iter().cloned());
        Self {
            patterns,
            min_chars: DEFAULT_MIN_CHARS,
        }
    }

    /// Create a filter for a specific source kind with hardcoded defaults.
    pub fn new_for_openclaw(extra_patterns: &[String]) -> Self {
        let source_defaults: Vec<String> = OPENCLAW_NOISE_PATTERNS
            .iter()
            .map(|s| s.to_string())
            .collect();
        Self::new_with_source_patterns(extra_patterns, &source_defaults)
    }

    /// Returns true if the content should be dropped (is noise).
    /// Checks minimum length first (fast path), then pattern matching.
    pub fn is_noise(&self, text: &str) -> bool {
        let trimmed = text.trim();

        // Fast path: too short.
        if trimmed.len() < self.min_chars {
            return true;
        }

        // Pattern matching (case-insensitive contains).
        let lower = trimmed.to_lowercase();
        for pattern in &self.patterns {
            if lower.contains(&pattern.to_lowercase()) {
                return true;
            }
        }

        false
    }

    /// Returns the number of configured patterns (excluding min_chars check).
    pub fn pattern_count(&self) -> usize {
        self.patterns.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_min_chars_filter() {
        let filter = NoiseFilter::new(&[]);
        assert!(filter.is_noise("short"));
        assert!(!filter.is_noise("This is a long enough string that should pass the minimum character check"));
    }

    #[test]
    fn test_pattern_matching() {
        let filter = NoiseFilter::new(&["HEARTBEAT_OK".to_string()]);
        assert!(filter.is_noise("HEARTBEAT_OK - system status ping at 14:32:05 from monitor agent running"));
        assert!(!filter.is_noise("The user prefers dark mode for all their coding tools and applications"));
    }

    #[test]
    fn test_openclaw_noise_patterns() {
        let filter = NoiseFilter::new_for_openclaw(&[]);
        assert!(filter.is_noise("HEARTBEAT_OK timestamp 2024-01-01 system check passed"));
        assert!(filter.is_noise("Token Monitor Report: 45000 tokens used in current context window"));
        assert!(filter.is_noise("Switchboard - Cross-Subagent routing to vita agent for task completion"));
        assert!(!filter.is_noise("User prefers Rust over Go for backend services due to memory safety guarantees"));
    }

    #[test]
    fn test_case_insensitive_matching() {
        let filter = NoiseFilter::new(&["heartbeat_ok".to_string()]);
        assert!(filter.is_noise("HEARTBEAT_OK - uppercase should still match lowercase pattern filter"));
    }
}

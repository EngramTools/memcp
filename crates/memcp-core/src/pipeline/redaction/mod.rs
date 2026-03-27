//! PII and secret redaction engine.
//!
//! Two-phase scanning: RegexSet screens content in one pass, then individual
//! Regex patterns extract and mask matches. Shannon entropy filtering rejects
//! placeholder values. Allowlists bypass redaction for known-safe content.

pub mod entropy;
pub mod rules;

use crate::config::RedactionConfig;
use crate::errors::MemcpError;
use regex::{Regex, RegexSet};
use rules::{MaskStyle, RedactionRule, RuleType};
use std::collections::HashSet;

/// Result of redacting content.
#[derive(Debug, Clone)]
pub struct RedactionResult {
    /// The (possibly redacted) content
    pub content: String,
    /// Number of individual redactions applied
    pub redaction_count: usize,
    /// Unique category names that were redacted
    pub categories: Vec<String>,
    /// Whether any redaction was applied
    pub was_redacted: bool,
}

impl RedactionResult {
    /// Create a clean (no redaction) result.
    pub fn clean(content: &str) -> Self {
        RedactionResult {
            content: content.to_string(),
            redaction_count: 0,
            categories: Vec::new(),
            was_redacted: false,
        }
    }
}

/// Two-phase regex-based redaction engine.
///
/// Phase 1: RegexSet screens content for any potential matches (fast, single pass).
/// Phase 2: Individual Regex patterns extract and mask each match.
///
/// Supports allowlists (exact values and patterns) to bypass redaction for
/// known-safe content like test fixtures.
pub struct RedactionEngine {
    /// Fast screening set — same order as `rules`
    screening_set: RegexSet,
    /// Individual rules with capture regexes
    rules: Vec<RedactionRule>,
    /// Exact values that bypass redaction
    allowlist_values: HashSet<String>,
    /// Compiled patterns that bypass redaction
    allowlist_patterns: Option<RegexSet>,
    /// Entropy threshold for generic secret detection
    entropy_threshold: f64,
}

impl RedactionEngine {
    /// Build a RedactionEngine from configuration.
    ///
    /// Fail-closed: if any regex fails to compile, returns an error
    /// (the store operation should be rejected).
    pub fn from_config(config: &RedactionConfig) -> Result<Self, MemcpError> {
        let mut all_rules = Vec::new();

        if config.secrets_enabled {
            all_rules.extend(rules::default_secret_rules());
        }

        if config.pii_enabled {
            all_rules.extend(rules::default_pii_rules());
        }

        // Add custom rules from config
        for custom in &config.custom_rules {
            let mask_style = match custom.mask_style.as_str() {
                "partial" => MaskStyle::Partial {
                    prefix_len: custom.prefix_len.unwrap_or(4),
                },
                _ => MaskStyle::Full,
            };
            let regex = Regex::new(&custom.pattern).map_err(|e| {
                MemcpError::Config(format!(
                    "Invalid custom redaction pattern '{}': {}",
                    custom.pattern, e
                ))
            })?;
            all_rules.push(RedactionRule {
                category: custom.category.clone(),
                regex,
                screening_pattern: custom.pattern.clone(),
                mask_style,
                entropy_check: false,
                rule_type: RuleType::Secret,
            });
        }

        let screening_patterns: Vec<&str> = all_rules
            .iter()
            .map(|r| r.screening_pattern.as_str())
            .collect();

        let screening_set = RegexSet::new(&screening_patterns).map_err(|e| {
            MemcpError::Config(format!("Failed to compile redaction screening set: {}", e))
        })?;

        let allowlist_values: HashSet<String> = config.allowlist.values.iter().cloned().collect();

        let allowlist_patterns = if config.allowlist.patterns.is_empty() {
            None
        } else {
            Some(
                RegexSet::new(&config.allowlist.patterns)
                    .map_err(|e| MemcpError::Config(format!("Invalid allowlist pattern: {}", e)))?,
            )
        };

        Ok(RedactionEngine {
            screening_set,
            rules: all_rules,
            allowlist_values,
            allowlist_patterns,
            entropy_threshold: config.entropy_threshold,
        })
    }

    /// Redact secrets and PII from content.
    ///
    /// Phase 1: RegexSet screening — if no patterns match, return immediately.
    /// Phase 2: For each matched rule, find individual matches, check allowlist
    /// and entropy, apply masking right-to-left to preserve byte offsets.
    pub fn redact(&self, content: &str) -> Result<RedactionResult, MemcpError> {
        // Phase 1: fast screening
        let matched_indices: Vec<usize> = self.screening_set.matches(content).into_iter().collect();
        if matched_indices.is_empty() {
            return Ok(RedactionResult::clean(content));
        }

        // Phase 2: collect all replacements
        let mut replacements: Vec<(usize, usize, String)> = Vec::new(); // (start, end, replacement)
        let mut categories: HashSet<String> = HashSet::new();

        for &rule_idx in &matched_indices {
            let rule = &self.rules[rule_idx];

            for cap in rule.regex.captures_iter(content) {
                // Use capture group 1 if it exists, otherwise group 0
                let m = cap.get(1).unwrap_or_else(|| cap.get(0).unwrap());
                let matched_text = m.as_str();

                // Skip if inside existing [REDACTED:...] marker
                if matched_text.starts_with("[REDACTED:") {
                    continue;
                }

                // Check allowlist exact values
                if self.allowlist_values.contains(matched_text) {
                    continue;
                }

                // Check allowlist patterns
                if let Some(ref patterns) = self.allowlist_patterns {
                    if patterns.is_match(matched_text) {
                        continue;
                    }
                }

                // Entropy check for generic patterns
                if rule.entropy_check {
                    let ent = entropy::shannon_entropy(matched_text);
                    if ent < self.entropy_threshold {
                        continue;
                    }
                }

                // Build replacement
                let replacement = match &rule.mask_style {
                    MaskStyle::Partial { prefix_len } => {
                        let prefix: String = matched_text.chars().take(*prefix_len).collect();
                        format!("{}[REDACTED:{}]", prefix, rule.category)
                    }
                    MaskStyle::Full => {
                        format!("[REDACTED:{}]", rule.category)
                    }
                };

                categories.insert(rule.category.clone());
                replacements.push((m.start(), m.end(), replacement));
            }
        }

        if replacements.is_empty() {
            return Ok(RedactionResult::clean(content));
        }

        // Deduplicate overlapping replacements (keep first encountered)
        replacements.sort_by_key(|(start, _, _)| *start);
        let mut deduped: Vec<(usize, usize, String)> = Vec::new();
        for r in replacements {
            if let Some(last) = deduped.last() {
                if r.0 < last.1 {
                    continue; // overlapping, skip
                }
            }
            deduped.push(r);
        }

        let redaction_count = deduped.len();

        // Apply replacements right-to-left to preserve byte offsets
        let mut result = content.to_string();
        for (start, end, replacement) in deduped.into_iter().rev() {
            result.replace_range(start..end, &replacement);
        }

        let mut cat_vec: Vec<String> = categories.into_iter().collect();
        cat_vec.sort();

        Ok(RedactionResult {
            content: result,
            redaction_count,
            categories: cat_vec,
            was_redacted: true,
        })
    }
}

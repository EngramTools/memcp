//! Default secret and PII pattern definitions for the redaction engine.
//!
//! Provides 13+ secret patterns (AWS, Anthropic, OpenAI, GitHub, Stripe, etc.)
//! and opt-in PII patterns (SSN, credit card). Email is intentionally excluded
//! per user decision.

use regex::Regex;

/// How to mask a detected secret or PII value.
#[derive(Debug, Clone)]
pub enum MaskStyle {
    /// Preserve first N characters, replace rest with [REDACTED:category]
    Partial { prefix_len: usize },
    /// Replace entire match with [REDACTED:category]
    Full,
}

/// Whether a rule detects secrets or PII.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleType {
    Secret,
    Pii,
}

/// A single redaction rule: regex pattern + metadata for masking.
pub struct RedactionRule {
    pub category: String,
    pub regex: Regex,
    /// Pattern string for RegexSet screening (may differ from capture regex)
    pub screening_pattern: String,
    pub mask_style: MaskStyle,
    /// Whether to check Shannon entropy on the captured value
    pub entropy_check: bool,
    pub rule_type: RuleType,
}

impl RedactionRule {
    /// Create a secret detection rule.
    pub fn secret(
        category: &str,
        pattern: &str,
        mask_style: MaskStyle,
        entropy_check: bool,
    ) -> Self {
        RedactionRule {
            category: category.to_string(),
            regex: Regex::new(pattern).expect("built-in secret pattern must compile"),
            screening_pattern: pattern.to_string(),
            mask_style,
            entropy_check,
            rule_type: RuleType::Secret,
        }
    }

    /// Create a PII detection rule.
    pub fn pii(category: &str, pattern: &str, mask_style: MaskStyle) -> Self {
        RedactionRule {
            category: category.to_string(),
            regex: Regex::new(pattern).expect("built-in PII pattern must compile"),
            screening_pattern: pattern.to_string(),
            mask_style,
            entropy_check: false,
            rule_type: RuleType::Pii,
        }
    }
}

/// Default secret detection rules (13 patterns).
///
/// Each rule uses partial masking to preserve the identifying prefix.
/// Generic secret pattern uses entropy check to avoid placeholder false positives.
pub fn default_secret_rules() -> Vec<RedactionRule> {
    vec![
        // AWS Access Key ID: AKIA followed by 16 alphanumeric chars
        RedactionRule::secret(
            "aws_key",
            r"(AKIA[0-9A-Z]{16})",
            MaskStyle::Partial { prefix_len: 4 },
            false,
        ),
        // Anthropic API key: sk-ant-api03-...
        RedactionRule::secret(
            "anthropic_key",
            r"(sk-ant-api03-[A-Za-z0-9_-]{20,})",
            MaskStyle::Partial { prefix_len: 13 },
            false,
        ),
        // OpenAI API key: sk-proj... or sk-<alphanumeric 20+> (but not sk-ant-)
        // Note: Anthropic keys match first (rules are ordered), so sk-ant- won't reach here.
        // We match sk- followed by a non-'a' char or 'a' followed by non-'n', etc.
        RedactionRule::secret(
            "openai_key",
            r"(sk-(?:proj|live|test|org-)[A-Za-z0-9_-]{20,})",
            MaskStyle::Partial { prefix_len: 3 },
            false,
        ),
        // GitHub Personal Access Token (classic): ghp_...
        RedactionRule::secret(
            "github_pat",
            r"(ghp_[A-Za-z0-9]{36})",
            MaskStyle::Partial { prefix_len: 4 },
            false,
        ),
        // GitHub Fine-grained PAT: github_pat_...
        RedactionRule::secret(
            "github_fine_pat",
            r"(github_pat_[A-Za-z0-9_]{22,})",
            MaskStyle::Partial { prefix_len: 11 },
            false,
        ),
        // Slack Bot/User token: xoxb-... or xoxp-...
        RedactionRule::secret(
            "slack_token",
            r"(xox[bp]-[0-9]{10,}-[0-9]{10,}-[A-Za-z0-9]{20,})",
            MaskStyle::Partial { prefix_len: 5 },
            false,
        ),
        // Stripe secret key: sk_live_... or sk_test_...
        RedactionRule::secret(
            "stripe_key",
            r"(sk_(?:live|test)_[A-Za-z0-9]{24,})",
            MaskStyle::Partial { prefix_len: 8 },
            false,
        ),
        // npm token: npm_...
        RedactionRule::secret(
            "npm_token",
            r"(npm_[A-Za-z0-9]{36})",
            MaskStyle::Partial { prefix_len: 4 },
            false,
        ),
        // GCP API key: AIza...
        RedactionRule::secret(
            "gcp_key",
            r"(AIza[A-Za-z0-9_-]{35})",
            MaskStyle::Partial { prefix_len: 4 },
            false,
        ),
        // Twilio API key: SK...
        RedactionRule::secret(
            "twilio_key",
            r"(SK[0-9a-f]{32})",
            MaskStyle::Partial { prefix_len: 2 },
            false,
        ),
        // HuggingFace token: hf_...
        RedactionRule::secret(
            "huggingface_token",
            r"(hf_[A-Za-z0-9]{34})",
            MaskStyle::Partial { prefix_len: 3 },
            false,
        ),
        // PyPI token: pypi-...
        RedactionRule::secret(
            "pypi_token",
            r"(pypi-[A-Za-z0-9_-]{50,})",
            MaskStyle::Partial { prefix_len: 5 },
            false,
        ),
        // Generic secret assignment: key/secret/token/password = <value>
        // Uses entropy check to filter out placeholders
        RedactionRule::secret(
            "generic_secret",
            r#"(?i)(?:secret|token|password|api_?key|apikey)\s*[=:]\s*["']?([A-Za-z0-9_\-/.+]{16,})["']?"#,
            MaskStyle::Full,
            true,
        ),
    ]
}

/// Default PII detection rules (SSN and credit card only).
///
/// Email is intentionally excluded per user decision.
/// Phone and IP are excluded to avoid false positives.
pub fn default_pii_rules() -> Vec<RedactionRule> {
    vec![
        // US Social Security Number: XXX-XX-XXXX
        RedactionRule::pii("ssn", r"\b(\d{3}-\d{2}-\d{4})\b", MaskStyle::Full),
        // Credit card number: 4 groups of 4 digits separated by dashes or spaces
        RedactionRule::pii(
            "credit_card",
            r"\b(\d{4}[-\s]?\d{4}[-\s]?\d{4}[-\s]?\d{4})\b",
            MaskStyle::Full,
        ),
    ]
}

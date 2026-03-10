//! Unit tests for RedactionEngine — secret detection, PII masking, entropy filtering, allowlists.

use memcp::config::{AllowlistConfig, CustomRuleConfig, RedactionConfig};
use memcp::pipeline::redaction::RedactionEngine;

fn secrets_only_config() -> RedactionConfig {
    RedactionConfig {
        secrets_enabled: true,
        pii_enabled: false,
        entropy_threshold: 3.5,
        allowlist: AllowlistConfig::default(),
        custom_rules: Vec::new(),
    }
}

fn secrets_and_pii_config() -> RedactionConfig {
    RedactionConfig {
        secrets_enabled: true,
        pii_enabled: true,
        entropy_threshold: 3.5,
        allowlist: AllowlistConfig::default(),
        custom_rules: Vec::new(),
    }
}

fn pii_only_config() -> RedactionConfig {
    RedactionConfig {
        secrets_enabled: false,
        pii_enabled: true,
        entropy_threshold: 3.5,
        allowlist: AllowlistConfig::default(),
        custom_rules: Vec::new(),
    }
}

// --- Secret detection tests ---

#[test]
fn test_anthropic_key_redacted() {
    let engine = RedactionEngine::from_config(&secrets_only_config()).unwrap();
    let result = engine.redact("my key is sk-ant-api03-AbCdEfGhIjKlMnOpQrStUv").unwrap();
    assert!(result.was_redacted);
    assert!(
        result.content.contains("sk-ant-api03-[REDACTED:anthropic_key]"),
        "expected partial mask with prefix, got: {}",
        result.content
    );
    assert_eq!(result.redaction_count, 1);
    assert!(result.categories.contains(&"anthropic_key".to_string()));
}

#[test]
fn test_aws_key_redacted() {
    let engine = RedactionEngine::from_config(&secrets_only_config()).unwrap();
    let result = engine.redact("AWS key AKIAIOSFODNN7EXAMPLE").unwrap();
    assert!(result.was_redacted);
    assert!(
        result.content.contains("AKIA[REDACTED:aws_key]"),
        "expected partial mask preserving AKIA prefix, got: {}",
        result.content
    );
}

#[test]
fn test_github_pat_redacted() {
    let engine = RedactionEngine::from_config(&secrets_only_config()).unwrap();
    let result = engine
        .redact("token ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef1234")
        .unwrap();
    assert!(result.was_redacted);
    assert!(
        result.content.contains("ghp_[REDACTED:github_pat]"),
        "expected partial mask preserving ghp_ prefix, got: {}",
        result.content
    );
}

#[test]
fn test_openai_key_redacted() {
    let engine = RedactionEngine::from_config(&secrets_only_config()).unwrap();
    let result = engine
        .redact("key sk-proj1234567890abcdefghij")
        .unwrap();
    assert!(result.was_redacted);
    assert!(
        result.content.contains("sk-[REDACTED:openai_key]"),
        "expected partial mask preserving sk- prefix, got: {}",
        result.content
    );
}

#[test]
fn test_stripe_key_redacted() {
    let engine = RedactionEngine::from_config(&secrets_only_config()).unwrap();
    let result = engine
        .redact("my stripe key is sk_live_abcdefghijklmnopqrstuvwx")
        .unwrap();
    assert!(result.was_redacted);
    assert!(
        result.content.contains("sk_live_[REDACTED:stripe_key]"),
        "expected partial mask preserving sk_live_ prefix, got: {}",
        result.content
    );
}

// --- PII detection tests ---

#[test]
fn test_pii_disabled_no_ssn_redaction() {
    let engine = RedactionEngine::from_config(&secrets_only_config()).unwrap();
    let result = engine.redact("my SSN is 123-45-6789").unwrap();
    assert!(!result.was_redacted, "PII should not be redacted when pii_enabled=false");
    assert!(result.content.contains("123-45-6789"));
}

#[test]
fn test_pii_enabled_ssn_redacted() {
    let engine = RedactionEngine::from_config(&secrets_and_pii_config()).unwrap();
    let result = engine.redact("SSN 123-45-6789").unwrap();
    assert!(result.was_redacted);
    assert!(
        result.content.contains("[REDACTED:ssn]"),
        "expected full mask for SSN, got: {}",
        result.content
    );
}

#[test]
fn test_pii_enabled_credit_card_redacted() {
    let engine = RedactionEngine::from_config(&secrets_and_pii_config()).unwrap();
    let result = engine.redact("card 4111-1111-1111-1111").unwrap();
    assert!(result.was_redacted);
    assert!(
        result.content.contains("[REDACTED:credit_card]"),
        "expected full mask for credit card, got: {}",
        result.content
    );
}

#[test]
fn test_email_never_redacted() {
    let engine = RedactionEngine::from_config(&secrets_and_pii_config()).unwrap();
    let result = engine.redact("contact user@example.com for details").unwrap();
    assert!(
        result.content.contains("user@example.com"),
        "email should never be redacted, got: {}",
        result.content
    );
}

// --- No secrets in clean content ---

#[test]
fn test_clean_content_no_redaction() {
    let engine = RedactionEngine::from_config(&secrets_only_config()).unwrap();
    let result = engine.redact("This is perfectly clean content with no secrets").unwrap();
    assert!(!result.was_redacted);
    assert_eq!(result.redaction_count, 0);
    assert!(result.categories.is_empty());
}

// --- Entropy filtering ---

#[test]
fn test_generic_secret_placeholder_not_redacted() {
    let engine = RedactionEngine::from_config(&secrets_only_config()).unwrap();
    let result = engine.redact("secret = your-api-key-here").unwrap();
    assert!(
        !result.was_redacted,
        "placeholder with low entropy should not be redacted, got: {}",
        result.content
    );
}

#[test]
fn test_generic_secret_real_value_redacted() {
    let engine = RedactionEngine::from_config(&secrets_only_config()).unwrap();
    let result = engine.redact("secret = aB3xK9mN2pQ7rT5w8yZ1").unwrap();
    assert!(
        result.was_redacted,
        "real secret with high entropy should be redacted, got: {}",
        result.content
    );
    assert!(
        result.content.contains("[REDACTED:generic_secret]"),
        "expected generic_secret category, got: {}",
        result.content
    );
}

// --- Allowlist tests ---

#[test]
fn test_allowlist_exact_value_bypasses_redaction() {
    let config = RedactionConfig {
        secrets_enabled: true,
        pii_enabled: false,
        entropy_threshold: 3.5,
        allowlist: AllowlistConfig {
            values: vec!["AKIAIOSFODNN7EXAMPLE".to_string()],
            patterns: Vec::new(),
        },
        custom_rules: Vec::new(),
    };
    let engine = RedactionEngine::from_config(&config).unwrap();
    let result = engine.redact("key AKIAIOSFODNN7EXAMPLE").unwrap();
    assert!(
        !result.was_redacted,
        "allowlisted value should bypass redaction, got: {}",
        result.content
    );
}

#[test]
fn test_allowlist_pattern_bypasses_redaction() {
    let config = RedactionConfig {
        secrets_enabled: true,
        pii_enabled: false,
        entropy_threshold: 3.5,
        allowlist: AllowlistConfig {
            values: Vec::new(),
            patterns: vec!["(?i)example".to_string()],
        },
        custom_rules: Vec::new(),
    };
    let engine = RedactionEngine::from_config(&config).unwrap();
    let result = engine.redact("key AKIAIOSFODNN7EXAMPLE").unwrap();
    assert!(
        !result.was_redacted,
        "allowlist pattern should bypass redaction, got: {}",
        result.content
    );
}

// --- Multiple secrets ---

#[test]
fn test_multiple_secrets_all_redacted() {
    let engine = RedactionEngine::from_config(&secrets_only_config()).unwrap();
    let content = "AWS AKIAIOSFODNN7EXAMPLE and token ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef1234";
    let result = engine.redact(content).unwrap();
    assert!(result.was_redacted);
    assert!(result.redaction_count >= 2, "expected at least 2 redactions, got {}", result.redaction_count);
    assert!(result.categories.contains(&"aws_key".to_string()));
    assert!(result.categories.contains(&"github_pat".to_string()));
}

// --- RedactionResult.categories ---

#[test]
fn test_categories_unique() {
    let engine = RedactionEngine::from_config(&secrets_and_pii_config()).unwrap();
    let content = "SSN 123-45-6789 and another SSN 987-65-4321";
    let result = engine.redact(content).unwrap();
    assert!(result.was_redacted);
    // categories should contain "ssn" exactly once despite two matches
    let ssn_count = result.categories.iter().filter(|c| *c == "ssn").count();
    assert_eq!(ssn_count, 1, "categories should be unique");
    assert_eq!(result.redaction_count, 2);
}

// --- Custom rules ---

#[test]
fn test_custom_rule_applied() {
    let config = RedactionConfig {
        secrets_enabled: false,
        pii_enabled: false,
        entropy_threshold: 3.5,
        allowlist: AllowlistConfig::default(),
        custom_rules: vec![CustomRuleConfig {
            pattern: r"(MYTOKEN_[A-Za-z0-9]{10,})".to_string(),
            category: "my_token".to_string(),
            mask_style: "partial".to_string(),
            prefix_len: Some(8),
        }],
    };
    let engine = RedactionEngine::from_config(&config).unwrap();
    let result = engine.redact("token is MYTOKEN_abcdefghij").unwrap();
    assert!(result.was_redacted);
    assert!(
        result.content.contains("MYTOKEN_[REDACTED:my_token]"),
        "custom rule should apply partial mask, got: {}",
        result.content
    );
}

#[test]
fn test_invalid_custom_rule_fails_closed() {
    let config = RedactionConfig {
        secrets_enabled: false,
        pii_enabled: false,
        entropy_threshold: 3.5,
        allowlist: AllowlistConfig::default(),
        custom_rules: vec![CustomRuleConfig {
            pattern: r"(invalid[".to_string(),
            category: "bad".to_string(),
            mask_style: "full".to_string(),
            prefix_len: None,
        }],
    };
    let result = RedactionEngine::from_config(&config);
    assert!(result.is_err(), "invalid regex should cause construction failure (fail-closed)");
}

// --- Disabled engine ---

#[test]
fn test_both_disabled_clean_passthrough() {
    let config = RedactionConfig {
        secrets_enabled: false,
        pii_enabled: false,
        entropy_threshold: 3.5,
        allowlist: AllowlistConfig::default(),
        custom_rules: Vec::new(),
    };
    let engine = RedactionEngine::from_config(&config).unwrap();
    let result = engine.redact("AKIAIOSFODNN7EXAMPLE and SSN 123-45-6789").unwrap();
    assert!(!result.was_redacted, "disabled engine should not redact anything");
}

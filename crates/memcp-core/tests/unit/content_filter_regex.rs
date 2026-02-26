use memcp::content_filter::regex_filter::RegexFilter;

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

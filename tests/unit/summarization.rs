use memcp::config::SummarizationConfig;
use memcp::summarization::create_summarization_provider;

#[test]
fn test_create_provider_disabled() {
    let config = SummarizationConfig::default(); // enabled: false
    let result = create_summarization_provider(&config).unwrap();
    assert!(result.is_none());
}

#[test]
fn test_create_provider_ollama() {
    let mut config = SummarizationConfig::default();
    config.enabled = true;
    let result = create_summarization_provider(&config).unwrap();
    assert!(result.is_some());
    assert_eq!(result.unwrap().model_name(), "llama3.2:3b");
}

#[test]
fn test_create_provider_openai_missing_key() {
    let mut config = SummarizationConfig::default();
    config.enabled = true;
    config.provider = "openai".to_string();
    let result = create_summarization_provider(&config);
    assert!(result.is_err());
}

#[test]
fn test_create_provider_openai_with_key() {
    let mut config = SummarizationConfig::default();
    config.enabled = true;
    config.provider = "openai".to_string();
    config.openai_api_key = Some("sk-test".to_string());
    let result = create_summarization_provider(&config).unwrap();
    assert!(result.is_some());
    assert_eq!(result.unwrap().model_name(), "gpt-4o-mini");
}

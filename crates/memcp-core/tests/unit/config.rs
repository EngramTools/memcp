use memcp::config::Config;

#[test]
fn test_config_defaults() {
    let config = Config::default();
    assert_eq!(config.log_level, "info");
    assert_eq!(config.log_file, None);
    assert_eq!(
        config.database_url,
        "postgres://memcp:memcp@localhost:5432/memcp"
    );
    assert_eq!(config.embedding.provider, "local");
    assert_eq!(config.embedding.openai_api_key, None);
    assert_eq!(config.search.bm25_backend, "native");
    assert_eq!(config.search.default_min_salience, None);
    assert!(!config.search.salience_hint_mode);
}

/// A single golden dataset entry for recall quality tests.
#[derive(serde::Deserialize)]
pub struct GoldenQuery {
    pub query: String,
    pub seed_content: String,
    pub expected_top_content: String,
    pub min_score: f32,
    #[serde(default)]
    pub category: String,
}

/// Load golden queries from the bundled fixture file.
pub fn load_golden_queries() -> Vec<GoldenQuery> {
    let json = include_str!("../fixtures/golden_queries.json");
    serde_json::from_str(json).expect("invalid golden_queries.json")
}

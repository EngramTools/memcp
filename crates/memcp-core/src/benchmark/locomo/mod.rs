/// LoCoMo benchmark module.
///
/// Provides typed dataset structs, dataset loading, F1 scoring, dual-mode ingestion,
/// and LLM answer generation for the LoCoMo benchmark.
pub mod dataset;
pub mod evaluate;
pub mod ingest;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// ─── Dataset Types ────────────────────────────────────────────────────────────

/// A single LoCoMo sample: one long conversation with associated QA pairs.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoCoMoSample {
    pub sample_id: String,
    /// The conversation sessions.
    pub conversation: Vec<Session>,
    /// QA pairs to evaluate against the conversation.
    pub qa: Vec<QaPair>,
}

/// One session (day) of conversation between two speakers.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Session {
    pub date: String,
    pub speakers: Vec<String>,
    /// Dialog turns — also accepted as "turns" for flexibility.
    #[serde(alias = "turns")]
    pub dialog: Vec<Turn>,
}

/// A single dialog turn.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Turn {
    pub speaker: String,
    pub dialog_id: usize,
    #[serde(alias = "text")]
    pub text: String,
}

/// A QA pair with flexible category deserialization (u8 or string in the wild).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QaPair {
    pub question: String,
    pub answer: String,
    /// Category: 1=single_hop, 2=multi_hop, 3=temporal, 4=commonsense, 5=adversarial.
    /// Stored as `Value` to handle both integer and string serializations.
    pub category: Value,
    /// dialog_ids of turns containing the answer evidence.
    #[serde(default)]
    pub evidence: Vec<usize>,
}

impl QaPair {
    /// Normalize category to u8 regardless of whether it was serialized as integer or string.
    pub fn category_u8(&self) -> u8 {
        match &self.category {
            Value::Number(n) => n.as_u64().unwrap_or(1) as u8,
            Value::String(s) => s.parse::<u8>().unwrap_or(1),
            _ => 1,
        }
    }
}

/// Map category numeric code to a human-readable label.
pub fn category_label(cat: u8) -> &'static str {
    match cat {
        1 => "single_hop",
        2 => "multi_hop",
        3 => "temporal",
        4 => "commonsense",
        5 => "adversarial",
        _ => "unknown",
    }
}

// ─── Ingestion Mode ───────────────────────────────────────────────────────────

/// How to ingest LoCoMo conversations into memcp memories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoCoMoIngestionMode {
    /// One memory per dialog turn: "{speaker}: {text}"
    PerTurn,
    /// One memory per session: concatenated turns with date prefix.
    PerSession,
}

// ─── Result Types ─────────────────────────────────────────────────────────────

/// Per-question result for a LoCoMo benchmark run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoCoMoQuestionResult {
    pub sample_id: String,
    pub question: String,
    pub answer: String,
    pub hypothesis: String,
    pub f1: f64,
    pub category: u8,
    pub latency_ms: u64,
}

/// Per-category F1 statistics for a LoCoMo run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoCoMoCategoryStats {
    pub mean_f1: f64,
    pub count: usize,
}

/// Full result set for a LoCoMo benchmark run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoCoMoResult {
    pub config_name: String,
    pub ingestion_mode: LoCoMoIngestionMode,
    pub overall_f1: f64,
    pub per_category: HashMap<String, LoCoMoCategoryStats>,
    pub question_count: usize,
    pub results: Vec<LoCoMoQuestionResult>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_category_label_mapping() {
        assert_eq!(category_label(1), "single_hop");
        assert_eq!(category_label(2), "multi_hop");
        assert_eq!(category_label(3), "temporal");
        assert_eq!(category_label(4), "commonsense");
        assert_eq!(category_label(5), "adversarial");
    }

    #[test]
    fn test_qa_pair_category_u8_from_number() {
        let qa = QaPair {
            question: "Q".into(),
            answer: "A".into(),
            category: json!(3u8),
            evidence: vec![],
        };
        assert_eq!(qa.category_u8(), 3);
    }

    #[test]
    fn test_qa_pair_category_u8_from_string() {
        let qa = QaPair {
            question: "Q".into(),
            answer: "A".into(),
            category: json!("2"),
            evidence: vec![],
        };
        assert_eq!(qa.category_u8(), 2);
    }

    #[test]
    fn test_locomo_sample_deserialize() {
        let json_str = r#"{
            "sample_id": "s001",
            "conversation": [
                {
                    "date": "March 15, 2023",
                    "speakers": ["Alice", "Bob"],
                    "dialog": [
                        {"speaker": "Alice", "dialog_id": 0, "text": "Hello Bob!"},
                        {"speaker": "Bob", "dialog_id": 1, "text": "Hi Alice!"}
                    ]
                }
            ],
            "qa": [
                {
                    "question": "What did Alice say?",
                    "answer": "Hello Bob",
                    "category": 1,
                    "evidence": [0]
                }
            ]
        }"#;
        let sample: LoCoMoSample = serde_json::from_str(json_str).expect("should deserialize");
        assert_eq!(sample.sample_id, "s001");
        assert_eq!(sample.conversation.len(), 1);
        assert_eq!(sample.conversation[0].dialog.len(), 2);
        assert_eq!(sample.qa.len(), 1);
        assert_eq!(sample.qa[0].category_u8(), 1);
    }
}

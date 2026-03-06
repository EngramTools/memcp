/// LoCoMo benchmark module.
///
/// Provides typed dataset structs, dataset loading, F1 scoring, dual-mode ingestion,
/// LLM answer generation, and the benchmark runner for the LoCoMo benchmark.
pub mod dataset;
pub mod evaluate;
pub mod ingest;
pub mod runner;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// ─── Dataset Types ────────────────────────────────────────────────────────────

/// A single LoCoMo sample: one long conversation with associated QA pairs.
///
/// The real `locomo10.json` stores `conversation` as a dict with keys like
/// `speaker_a`, `speaker_b`, `session_1`, `session_1_date_time`, etc.
/// The custom deserializer flattens this into `Vec<Session>`.
#[derive(Debug, Clone, Serialize)]
pub struct LoCoMoSample {
    pub sample_id: String,
    /// The conversation sessions (flattened from the dict format).
    pub conversation: Vec<Session>,
    /// QA pairs to evaluate against the conversation.
    pub qa: Vec<QaPair>,
}

/// One session (day) of conversation between two speakers.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Session {
    pub date: String,
    pub speakers: Vec<String>,
    /// Dialog turns.
    #[serde(alias = "turns")]
    pub dialog: Vec<Turn>,
}

/// A single dialog turn.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Turn {
    pub speaker: String,
    /// Dialog ID — string like "D1:3" in the real dataset.
    #[serde(alias = "dialog_id")]
    pub dia_id: String,
    pub text: String,
}

/// Deserialize an optional value that may be a string, number, or missing into a String.
fn deserialize_optional_string_or_number<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<Value>::deserialize(deserializer)?;
    match v {
        Some(Value::String(s)) => Ok(s),
        Some(Value::Number(n)) => Ok(n.to_string()),
        Some(other) => Ok(other.to_string()),
        None => Ok(String::new()),
    }
}

/// A QA pair with flexible category deserialization (u8 or string in the wild).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct QaPair {
    pub question: String,
    /// Answer text. Optional because category 5 (adversarial) uses `adversarial_answer` instead.
    /// Deserialized flexibly — the dataset sometimes uses integers (e.g. `2022`).
    #[serde(default, deserialize_with = "deserialize_optional_string_or_number")]
    pub answer: String,
    /// Adversarial answer — present only on category 5 questions (the "trick" answer).
    #[serde(default)]
    pub adversarial_answer: Option<String>,
    /// Category: 1=single_hop, 2=multi_hop, 3=temporal, 4=commonsense, 5=adversarial.
    /// Stored as `Value` to handle both integer and string serializations.
    pub category: Value,
    /// Evidence references (e.g. "D1:3" meaning dialog 1, turn 3).
    #[serde(default)]
    pub evidence: Vec<Value>,
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

// ─── Custom Deserialization ──────────────────────────────────────────────────

/// Raw JSON shape for a LoCoMo sample as it appears in `locomo10.json`.
///
/// `conversation` is a dict with:
///   - `speaker_a`, `speaker_b`: speaker names
///   - `session_N_date_time`: date string for session N
///   - `session_N`: list of turn objects `{speaker, dia_id, text, ...}`
#[derive(Deserialize)]
struct RawLoCoMoSample {
    sample_id: String,
    conversation: HashMap<String, Value>,
    qa: Vec<QaPair>,
}

impl<'de> Deserialize<'de> for LoCoMoSample {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawLoCoMoSample::deserialize(deserializer)?;

        let speaker_a = raw
            .conversation
            .get("speaker_a")
            .and_then(|v| v.as_str())
            .unwrap_or("Speaker A")
            .to_string();
        let speaker_b = raw
            .conversation
            .get("speaker_b")
            .and_then(|v| v.as_str())
            .unwrap_or("Speaker B")
            .to_string();
        let speakers = vec![speaker_a, speaker_b];

        // Collect session numbers by scanning for `session_N` keys (not `session_N_date_time`).
        let mut session_nums: Vec<usize> = Vec::new();
        for key in raw.conversation.keys() {
            if let Some(rest) = key.strip_prefix("session_") {
                if !rest.contains("date_time") {
                    if let Ok(n) = rest.parse::<usize>() {
                        session_nums.push(n);
                    }
                }
            }
        }
        session_nums.sort();

        let mut sessions = Vec::with_capacity(session_nums.len());
        for n in session_nums {
            let date_key = format!("session_{}_date_time", n);
            let session_key = format!("session_{}", n);

            let date = raw
                .conversation
                .get(&date_key)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let turns_value = raw
                .conversation
                .get(&session_key)
                .cloned()
                .unwrap_or(Value::Array(vec![]));

            let turns: Vec<Turn> = serde_json::from_value(turns_value).map_err(|e| {
                serde::de::Error::custom(format!("failed to parse session_{} turns: {}", n, e))
            })?;

            sessions.push(Session {
                date,
                speakers: speakers.clone(),
                dialog: turns,
            });
        }

        Ok(LoCoMoSample {
            sample_id: raw.sample_id,
            conversation: sessions,
            qa: raw.qa,
        })
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

// ─── Checkpoint State ─────────────────────────────────────────────────────────

/// Checkpoint/resume state for a LoCoMo benchmark run.
///
/// Saved after each sample completes so interrupted runs can resume from the
/// last completed sample (not from within a sample).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoCoMoState {
    /// Name of the BenchmarkConfig being run.
    pub config_name: String,
    /// Ingestion mode string ("per-turn" or "per-session").
    pub ingestion_mode: String,
    /// Sample IDs that have fully completed (all QA pairs evaluated).
    pub completed_sample_ids: Vec<String>,
    /// All question results accumulated so far.
    pub results: Vec<LoCoMoQuestionResult>,
    /// When this run started.
    pub started_at: DateTime<Utc>,
}

/// Save a LoCoMo checkpoint to disk.
pub fn save_locomo_checkpoint(
    state: &LoCoMoState,
    path: &std::path::Path,
) -> Result<(), anyhow::Error> {
    let json = serde_json::to_string_pretty(state)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Load a LoCoMo checkpoint from disk. Returns None if the file does not exist.
pub fn load_locomo_checkpoint(
    path: &std::path::Path,
) -> Result<Option<LoCoMoState>, anyhow::Error> {
    if path.exists() {
        let json = std::fs::read_to_string(path)?;
        let state: LoCoMoState = serde_json::from_str(&json)?;
        Ok(Some(state))
    } else {
        Ok(None)
    }
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
            evidence: vec![json!("D1:3")],
            adversarial_answer: None,
        };
        assert_eq!(qa.category_u8(), 3);
    }

    #[test]
    fn test_qa_pair_category_u8_from_string() {
        let qa = QaPair {
            question: "Q".into(),
            answer: "A".into(),
            category: json!("2"),
            evidence: vec![json!("D1:0")],
            adversarial_answer: None,
        };
        assert_eq!(qa.category_u8(), 2);
    }

    #[test]
    fn test_locomo_sample_deserialize_real_format() {
        let json_str = r#"{
            "sample_id": "s001",
            "conversation": {
                "speaker_a": "Alice",
                "speaker_b": "Bob",
                "session_1_date_time": "1:56 pm on 8 May, 2023",
                "session_1": [
                    {"speaker": "Alice", "dia_id": "D1:1", "text": "Hello Bob!"},
                    {"speaker": "Bob", "dia_id": "D1:2", "text": "Hi Alice!"}
                ],
                "session_2_date_time": "3:00 pm on 15 May, 2023",
                "session_2": [
                    {"speaker": "Alice", "dia_id": "D2:1", "text": "How are you?"}
                ]
            },
            "qa": [
                {
                    "question": "What did Alice say first?",
                    "answer": "Hello Bob",
                    "category": 1,
                    "evidence": ["D1:1"]
                }
            ]
        }"#;
        let sample: LoCoMoSample = serde_json::from_str(json_str).expect("should deserialize");
        assert_eq!(sample.sample_id, "s001");
        assert_eq!(sample.conversation.len(), 2);
        assert_eq!(sample.conversation[0].date, "1:56 pm on 8 May, 2023");
        assert_eq!(sample.conversation[0].speakers, vec!["Alice", "Bob"]);
        assert_eq!(sample.conversation[0].dialog.len(), 2);
        assert_eq!(sample.conversation[0].dialog[0].dia_id, "D1:1");
        assert_eq!(sample.conversation[1].dialog.len(), 1);
        assert_eq!(sample.qa.len(), 1);
        assert_eq!(sample.qa[0].category_u8(), 1);
    }

    #[test]
    fn test_locomo_sample_real_dataset_first_record() {
        // Minimal reproduction of the real locomo10.json structure
        let json_str = r#"{
            "sample_id": "test_real",
            "conversation": {
                "speaker_a": "Caroline",
                "speaker_b": "Melanie",
                "session_1_date_time": "1:56 pm on 8 May, 2023",
                "session_1": [
                    {"speaker": "Caroline", "dia_id": "D1:1", "text": "Hey Mel!"},
                    {"speaker": "Melanie", "dia_id": "D1:2", "text": "Hi Caroline!"},
                    {"speaker": "Caroline", "dia_id": "D1:3", "text": "Went to LGBTQ group yesterday."}
                ]
            },
            "qa": [
                {
                    "question": "When did Caroline go to the LGBTQ support group?",
                    "answer": "7 May 2023",
                    "evidence": ["D1:3"],
                    "category": 2
                }
            ]
        }"#;
        let sample: LoCoMoSample = serde_json::from_str(json_str).expect("should deserialize");
        assert_eq!(sample.conversation.len(), 1);
        assert_eq!(sample.conversation[0].dialog[2].dia_id, "D1:3");
    }
}

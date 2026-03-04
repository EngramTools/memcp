/// LoCoMo evaluation: F1 scoring and LLM answer generation.
///
/// Provides SQuAD-style token-level F1 scoring and LLM answer generation
/// for LoCoMo benchmark questions using the shared OpenAI retry utility.

use std::collections::HashMap;

use reqwest::Client;
use serde_json::json;

use crate::store::Memory;

use super::super::evaluate::{call_openai_with_retry, ANSWER_MODEL};

// ─── F1 Scoring ───────────────────────────────────────────────────────────────

/// Compute SQuAD-style token-level F1 between prediction and ground truth.
///
/// Returns 1.0 for exact match, 0.0 for no overlap.
/// Both strings are normalized (lowercased, punctuation stripped) before tokenizing.
pub fn f1_score(prediction: &str, ground_truth: &str) -> f64 {
    let pred_tokens = normalize_and_tokenize(prediction);
    let truth_tokens = normalize_and_tokenize(ground_truth);

    if pred_tokens.is_empty() && truth_tokens.is_empty() {
        return 1.0;
    }
    if pred_tokens.is_empty() || truth_tokens.is_empty() {
        return 0.0;
    }

    let pred_counts = token_counts(&pred_tokens);
    let truth_counts = token_counts(&truth_tokens);

    let common: usize = pred_counts
        .iter()
        .map(|(t, &c)| c.min(*truth_counts.get(t).unwrap_or(&0)))
        .sum();

    if common == 0 {
        return 0.0;
    }

    let precision = common as f64 / pred_tokens.len() as f64;
    let recall = common as f64 / truth_tokens.len() as f64;
    2.0 * precision * recall / (precision + recall)
}

/// Normalize a string for F1 scoring: lowercase and replace non-alphanumeric with spaces.
pub fn normalize_and_tokenize(s: &str) -> Vec<String> {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(String::from)
        .collect()
}

/// Count token occurrences (multiset) for F1 intersection computation.
pub fn token_counts(tokens: &[String]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for t in tokens {
        *counts.entry(t.clone()).or_insert(0) += 1;
    }
    counts
}

// ─── LLM Answer Generation ────────────────────────────────────────────────────

/// Build a LoCoMo-specific answer prompt from retrieved memories and a question.
fn build_locomo_answer_prompt(question: &str, memories: &[Memory]) -> String {
    let mut context_parts = Vec::new();
    for (i, m) in memories.iter().enumerate() {
        context_parts.push(format!("[Memory {}]: {}", i + 1, m.content));
    }
    let context = if context_parts.is_empty() {
        "No relevant memories found.".to_string()
    } else {
        context_parts.join("\n")
    };

    format!(
        "You are a helpful assistant with access to conversation memories.\n\
         Answer the question based on the provided memories. Be concise.\n\
         If the answer cannot be found in the memories, say \"I don't know\".\n\n\
         Memories:\n{context}\n\n\
         Question: {question}\n\n\
         Answer:"
    )
}

/// Generate an answer for a LoCoMo question from retrieved memories using the answer model.
pub async fn generate_locomo_answer(
    client: &Client,
    api_key: &str,
    question: &str,
    memories: &[Memory],
) -> Result<String, anyhow::Error> {
    let prompt = build_locomo_answer_prompt(question, memories);

    let body = json!({
        "model": ANSWER_MODEL,
        "temperature": 0,
        "max_tokens": 256,
        "messages": [{"role": "user", "content": prompt}]
    });

    let response_text = call_openai_with_retry(client, api_key, &body).await?;
    Ok(response_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_f1_exact_match() {
        assert_eq!(f1_score("Paris", "Paris"), 1.0);
    }

    #[test]
    fn test_f1_no_overlap() {
        assert_eq!(f1_score("London", "Paris"), 0.0);
    }

    #[test]
    fn test_f1_both_empty() {
        assert_eq!(f1_score("", ""), 1.0);
    }

    #[test]
    fn test_f1_one_empty() {
        assert_eq!(f1_score("something", ""), 0.0);
        assert_eq!(f1_score("", "something"), 0.0);
    }

    #[test]
    fn test_f1_partial_overlap() {
        // "The capital is Paris" vs "Paris": 1 common token out of 4 predicted / 1 ground truth.
        // precision=0.25, recall=1.0, F1 = 2*0.25*1.0/(0.25+1.0) = 0.4
        let score = f1_score("The capital is Paris", "Paris");
        assert!(score > 0.0, "Expected F1 > 0.0 (partial overlap), got {}", score);
        assert!(score < 1.0, "Expected F1 < 1.0, got {}", score);

        // "Paris France" vs "Paris": precision=0.5, recall=1.0, F1=2/3 > 0.5
        let score2 = f1_score("Paris France", "Paris");
        assert!(score2 > 0.5, "Expected F1 > 0.5 for 'Paris France' vs 'Paris', got {}", score2);
    }

    #[test]
    fn test_f1_punctuation_normalization() {
        // "Paris." and "paris" should match after normalization
        assert_eq!(f1_score("Paris.", "paris"), 1.0);
    }

    #[test]
    fn test_f1_case_insensitive() {
        assert_eq!(f1_score("LONDON", "london"), 1.0);
    }

    #[test]
    fn test_normalize_and_tokenize_strips_punctuation() {
        let tokens = normalize_and_tokenize("Hello, World!");
        assert_eq!(tokens, vec!["hello", "world"]);
    }

    #[test]
    fn test_normalize_and_tokenize_empty() {
        let tokens = normalize_and_tokenize("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_token_counts_multiset() {
        let tokens = vec!["a".to_string(), "b".to_string(), "a".to_string()];
        let counts = token_counts(&tokens);
        assert_eq!(counts["a"], 2);
        assert_eq!(counts["b"], 1);
    }
}

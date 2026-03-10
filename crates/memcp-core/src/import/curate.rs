//! Tier 2 LLM triage for import curation.
//!
//! `ImportCurator` classifies surviving import chunks using an LLM provider,
//! deciding whether to keep, skip, or merge each chunk, and assigning type_hints.
//!
//! Reuses the existing `SummarizationProvider` infrastructure — no new config needed.
//! If no LLM provider is configured, curation degrades gracefully (warn + skip).
//!
//! For conversation sources (ChatGPT, Claude.ai) the curator summarizes each
//! conversation into a single distilled memory rather than classifying chunks.

use std::sync::Arc;

use anyhow::Result;

use crate::config::Config;
use crate::pipeline::summarization::{create_summarization_provider, SummarizationProvider};

use super::ImportChunk;

// ── Decision types ────────────────────────────────────────────────────────────

/// Action decided by the LLM for a single chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CurationAction {
    /// Keep the chunk, store it with the given type_hint.
    Keep,
    /// Drop this chunk — noise or irrelevant.
    Skip,
    /// Merge with adjacent chunks (future: carry merge_group_id).
    Merge,
}

/// LLM curation decision for a single import chunk.
#[derive(Debug, Clone)]
pub struct CurationDecision {
    pub action: CurationAction,
    pub type_hint: String,
    pub topic: Option<String>,
}

impl Default for CurationDecision {
    fn default() -> Self {
        Self {
            action: CurationAction::Keep,
            type_hint: "observation".to_string(),
            topic: None,
        }
    }
}

// ── Curator ───────────────────────────────────────────────────────────────────

/// Tier 2 LLM triage. Holds a reference to the configured summarization provider.
pub struct ImportCurator {
    provider: Arc<dyn SummarizationProvider>,
}

/// Number of chunks per LLM classification batch.
const BATCH_SIZE: usize = 50;

impl ImportCurator {
    /// Construct a curator from the application config.
    ///
    /// Returns `None` if no summarization provider is configured (disabled or missing config).
    /// Logs a warning and returns None rather than returning an error — caller should
    /// continue without curation.
    pub fn new(config: &Config) -> Option<Self> {
        match create_summarization_provider(&config.summarization) {
            Ok(Some(provider)) => Some(Self { provider }),
            Ok(None) => {
                tracing::warn!(
                    "LLM provider not configured — skipping Tier 2 curation. \
                     Enable summarization in memcp.toml to use --curate."
                );
                None
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to create LLM provider for curation: {} — skipping Tier 2 curation.",
                    e
                );
                None
            }
        }
    }

    /// Classify a batch of import chunks using the LLM.
    ///
    /// Sends chunks in batches of `BATCH_SIZE` to keep prompts manageable.
    /// On LLM or parse errors, defaults to `keep observation` (fail-open).
    pub async fn classify_batch(&self, chunks: &[ImportChunk]) -> Result<Vec<CurationDecision>> {
        let mut all_decisions: Vec<CurationDecision> = Vec::with_capacity(chunks.len());

        for batch in chunks.chunks(BATCH_SIZE) {
            let decisions = self.classify_one_batch(batch).await;
            all_decisions.extend(decisions);
        }

        Ok(all_decisions)
    }

    /// Classify a single batch (up to BATCH_SIZE chunks).
    async fn classify_one_batch(&self, chunks: &[ImportChunk]) -> Vec<CurationDecision> {
        let prompt = build_classification_prompt(chunks);

        let response = match self.provider.summarize(&prompt).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    "LLM classification call failed: {} — defaulting to keep all",
                    e
                );
                return vec![CurationDecision::default(); chunks.len()];
            }
        };

        parse_classification_response(&response, chunks.len())
    }

    /// Summarize a full conversation into a distilled memory string.
    ///
    /// Used for ChatGPT and Claude.ai sources when `--curate` is set:
    /// instead of chunking, each conversation becomes one summarized memory.
    pub async fn summarize_conversation(&self, content: &str, title: &str) -> Result<String> {
        let prompt = format!(
            "Summarize this AI conversation into a concise memory. \
             Focus on: decisions made, preferences expressed, facts learned, instructions given. \
             Be specific and concrete. Omit pleasantries and filler.\n\
             Title: {}\n\n{}",
            title, content
        );

        let summary = self
            .provider
            .summarize(&prompt)
            .await
            .map_err(|e| anyhow::anyhow!("LLM summarization failed: {}", e))?;

        Ok(summary)
    }
}

// ── Prompt helpers ─────────────────────────────────────────────────────────────

/// Build the classification prompt for a batch of chunks.
fn build_classification_prompt(chunks: &[ImportChunk]) -> String {
    let mut prompt = String::from(
        "You are classifying imported AI conversation chunks for a memory system. \
         For each numbered chunk below, respond with EXACTLY one line in this format:\n\
         <number> <action> <type_hint> [optional topic]\n\n\
         Actions: keep | skip | merge\n\
         Type hints: fact | preference | instruction | decision | observation\n\n\
         Rules:\n\
         - keep: substantive content worth remembering (decisions, preferences, facts, instructions)\n\
         - skip: noise, pleasantries, off-topic, or ephemeral status messages\n\
         - merge: short fragment that belongs with adjacent content\n\n\
         Examples:\n\
         1 keep preference dark-mode\n\
         2 skip \n\
         3 keep decision rust-vs-go\n\
         4 merge observation \n\n\
         Chunks:\n",
    );

    for (i, chunk) in chunks.iter().enumerate() {
        let preview = chunk.content.chars().take(300).collect::<String>();
        prompt.push_str(&format!("\n[{}]\n{}\n", i + 1, preview));
    }

    prompt
}

/// Parse LLM classification response lines into `CurationDecision` values.
///
/// Expected format per line: `<number> <action> <type_hint> [topic]`
/// Falls back to `keep observation` on any parse failure.
fn parse_classification_response(response: &str, expected: usize) -> Vec<CurationDecision> {
    let mut decisions: Vec<Option<CurationDecision>> = vec![None; expected];

    for line in response.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.splitn(4, ' ').collect();
        if parts.is_empty() {
            continue;
        }

        // Parse chunk index (1-based in prompt, 0-based in Vec).
        let idx = match parts[0].parse::<usize>() {
            Ok(n) if n >= 1 && n <= expected => n - 1,
            _ => continue,
        };

        let action = parts
            .get(1)
            .map(|s| parse_action(s))
            .unwrap_or(CurationAction::Keep);
        let type_hint = parts
            .get(2)
            .filter(|s| is_valid_type_hint(s))
            .map(|s| s.to_string())
            .unwrap_or_else(|| "observation".to_string());
        let topic = parts
            .get(3)
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string());

        decisions[idx] = Some(CurationDecision {
            action,
            type_hint,
            topic,
        });
    }

    // Fill any unset positions with the default.
    decisions
        .into_iter()
        .map(|d| d.unwrap_or_default())
        .collect()
}

fn parse_action(s: &str) -> CurationAction {
    match s.to_lowercase().as_str() {
        "keep" => CurationAction::Keep,
        "skip" => CurationAction::Skip,
        "merge" => CurationAction::Merge,
        _ => CurationAction::Keep, // fail-open
    }
}

fn is_valid_type_hint(s: &str) -> bool {
    matches!(
        s,
        "fact" | "preference" | "instruction" | "decision" | "observation"
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_classification_response_basic() {
        let response = "1 keep preference dark-mode\n2 skip \n3 keep decision rust-lang";
        let decisions = parse_classification_response(response, 3);
        assert_eq!(decisions.len(), 3);

        assert_eq!(decisions[0].action, CurationAction::Keep);
        assert_eq!(decisions[0].type_hint, "preference");
        assert_eq!(decisions[0].topic, Some("dark-mode".to_string()));

        assert_eq!(decisions[1].action, CurationAction::Skip);

        assert_eq!(decisions[2].action, CurationAction::Keep);
        assert_eq!(decisions[2].type_hint, "decision");
        assert_eq!(decisions[2].topic, Some("rust-lang".to_string()));
    }

    #[test]
    fn test_parse_classification_response_defaults_on_bad_input() {
        let response = "not valid at all";
        let decisions = parse_classification_response(response, 2);
        assert_eq!(decisions.len(), 2);
        // Both default to keep observation.
        for d in &decisions {
            assert_eq!(d.action, CurationAction::Keep);
            assert_eq!(d.type_hint, "observation");
        }
    }

    #[test]
    fn test_parse_action() {
        assert_eq!(parse_action("keep"), CurationAction::Keep);
        assert_eq!(parse_action("skip"), CurationAction::Skip);
        assert_eq!(parse_action("merge"), CurationAction::Merge);
        assert_eq!(parse_action("KEEP"), CurationAction::Keep);
        assert_eq!(parse_action("unknown"), CurationAction::Keep); // fail-open
    }

    #[test]
    fn test_is_valid_type_hint() {
        assert!(is_valid_type_hint("fact"));
        assert!(is_valid_type_hint("preference"));
        assert!(is_valid_type_hint("instruction"));
        assert!(is_valid_type_hint("decision"));
        assert!(is_valid_type_hint("observation"));
        assert!(!is_valid_type_hint("memory"));
        assert!(!is_valid_type_hint("unknown"));
    }

    #[test]
    fn test_build_classification_prompt_contains_chunks() {
        let chunks = vec![ImportChunk {
            content: "User prefers dark mode for all editors.".to_string(),
            type_hint: None,
            source: "test".to_string(),
            tags: vec![],
            created_at: None,
            actor: None,
            embedding: None,
            embedding_model: None,
            project: None,
        }];
        let prompt = build_classification_prompt(&chunks);
        assert!(prompt.contains("[1]"));
        assert!(prompt.contains("dark mode"));
        assert!(prompt.contains("keep | skip | merge"));
    }

    #[test]
    fn test_curation_decision_default() {
        let d = CurationDecision::default();
        assert_eq!(d.action, CurationAction::Keep);
        assert_eq!(d.type_hint, "observation");
        assert!(d.topic.is_none());
    }
}

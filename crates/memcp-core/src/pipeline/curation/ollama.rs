//! Ollama-backed curation provider.
//!
//! Uses local Ollama LLM for contradiction detection within clusters
//! and intelligent merge synthesis. Follows the same HTTP pattern as
//! summarization/ollama.rs.

use async_trait::async_trait;
use serde::Deserialize;

use super::{ClusterMember, CurationAction, CurationError, CurationProvider};

/// Ollama-backed curation provider for LLM-powered memory review.
pub struct OllamaCurationProvider {
    base_url: String,
    model: String,
}

impl OllamaCurationProvider {
    pub fn new(base_url: String, model: String) -> Self {
        Self { base_url, model }
    }

    /// Format cluster members for LLM prompt.
    pub(crate) fn format_cluster(cluster: &[ClusterMember]) -> String {
        cluster
            .iter()
            .map(|m| {
                format!(
                    "[ID: {}] (stability: {:.2}, age: {}d, reinforced: {}x)\n{}",
                    m.id,
                    m.stability,
                    (chrono::Utc::now() - m.created_at).num_days(),
                    m.reinforcement_count,
                    m.content,
                )
            })
            .collect::<Vec<_>>()
            .join("\n---\n")
    }

    /// Parse LLM response into curation actions. Fail-open: bad response → all Skip.
    pub(crate) fn parse_review_response(
        response: &str,
        cluster: &[ClusterMember],
    ) -> Vec<CurationAction> {
        // Try to parse as JSON array
        #[derive(Deserialize)]
        struct LlmAction {
            action: String,
            #[serde(default)]
            source_ids: Vec<String>,
            #[serde(default)]
            memory_id: Option<String>,
            #[serde(default)]
            reason: Option<String>,
        }

        // Try extracting JSON from response (may have markdown wrapping)
        let json_str = response
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        match serde_json::from_str::<Vec<LlmAction>>(json_str) {
            Ok(llm_actions) => {
                llm_actions
                    .into_iter()
                    .filter_map(|a| {
                        let reason = a.reason.unwrap_or_default();
                        match a.action.as_str() {
                            "merge" => Some(CurationAction::Merge {
                                source_ids: a.source_ids,
                                synthesized_content: String::new(), // Will be filled by synthesize_merge
                            }),
                            "flag-stale" | "flag_stale" => {
                                a.memory_id.map(|id| CurationAction::FlagStale {
                                    memory_id: id,
                                    reason,
                                })
                            }
                            "strengthen" => {
                                a.memory_id.map(|id| CurationAction::Strengthen {
                                    memory_id: id,
                                    reason,
                                })
                            }
                            "skip" => {
                                a.memory_id.map(|id| CurationAction::Skip {
                                    memory_id: id,
                                    reason,
                                })
                            }
                            _ => None,
                        }
                    })
                    .collect()
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to parse LLM curation response — falling back to skip all");
                cluster
                    .iter()
                    .map(|m| CurationAction::Skip {
                        memory_id: m.id.clone(),
                        reason: "LLM response unparseable".to_string(),
                    })
                    .collect()
            }
        }
    }

    /// Call Ollama chat API.
    async fn chat(&self, system: &str, user: &str) -> Result<String, CurationError> {
        let client = reqwest::Client::new();
        let url = format!("{}/api/chat", self.base_url);

        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            "stream": false,
        });

        let response = client
            .post(&url)
            .json(&body)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| CurationError::Llm(format!("Ollama request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            return Err(CurationError::Llm(format!(
                "Ollama API error (status {}): {}",
                status, text
            )));
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| CurationError::Llm(format!("Failed to parse Ollama response: {}", e)))?;

        json["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| CurationError::Llm("No content in Ollama response".to_string()))
    }
}

pub(crate) const REVIEW_SYSTEM_PROMPT: &str = r#"You are reviewing a cluster of related memories from an AI's knowledge base.
For each memory, decide ONE action: merge, flag-stale, strengthen, or skip.

Rules:
- MERGE: If memories cover the same topic and can be combined without loss. List source IDs.
- FLAG-STALE: If a newer memory contradicts or supersedes an older one. Flag the OLDER one.
- STRENGTHEN: If a memory is frequently accessed and clearly valuable.
- SKIP: If no action is needed.

Respond with valid JSON array of actions:
[{"action": "merge", "source_ids": ["id1", "id2"], "reason": "..."},
 {"action": "flag-stale", "memory_id": "id1", "reason": "contradicted by id2"},
 {"action": "strengthen", "memory_id": "id1", "reason": "frequently accessed, core knowledge"},
 {"action": "skip", "memory_id": "id1", "reason": "..."}]

Only valid JSON. No markdown, no explanation outside JSON."#;

pub(crate) const MERGE_SYSTEM_PROMPT: &str = r#"Combine these related memories into a single concise entry.
Preserve all unique facts, decisions, and details.
Remove redundancy and contradictions (prefer newer information).
Output only the merged content, no commentary."#;

#[async_trait]
impl CurationProvider for OllamaCurationProvider {
    async fn review_cluster(
        &self,
        cluster: &[ClusterMember],
    ) -> Result<Vec<CurationAction>, CurationError> {
        let formatted = Self::format_cluster(cluster);
        let response = self.chat(REVIEW_SYSTEM_PROMPT, &formatted).await?;
        Ok(Self::parse_review_response(&response, cluster))
    }

    async fn synthesize_merge(
        &self,
        sources: &[ClusterMember],
    ) -> Result<String, CurationError> {
        let formatted = Self::format_cluster(sources);
        self.chat(MERGE_SYSTEM_PROMPT, &formatted).await
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    fn make_member(id: &str, stability: f64, trust: f32) -> ClusterMember {
        ClusterMember {
            id: id.to_string(),
            content: format!("Content for {}", id),
            type_hint: Some("fact".to_string()),
            tags: vec!["test".to_string()],
            created_at: Utc::now() - Duration::days(5),
            updated_at: Utc::now() - Duration::days(5),
            stability,
            reinforcement_count: 2,
            last_reinforced_at: Some(Utc::now() - Duration::days(1)),
            trust_level: trust,
        }
    }

    #[test]
    fn test_parse_suspicious_action() {
        let cluster = vec![make_member("id1", 2.0, 0.5)];
        let response = r#"[{"action": "suspicious", "memory_id": "id1", "reason": "override directive", "signals": ["override_instruction"]}]"#;
        let actions = OllamaCurationProvider::parse_review_response(response, &cluster);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            CurationAction::Suspicious { memory_id, reason, signals } => {
                assert_eq!(memory_id, "id1");
                assert_eq!(reason, "override directive");
                assert_eq!(signals, &vec!["override_instruction".to_string()]);
            }
            other => panic!("Expected Suspicious, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_unknown_action_skipped() {
        let cluster = vec![make_member("id1", 2.0, 0.5)];
        let response = r#"[{"action": "unknown_action", "memory_id": "id1", "reason": "test"}]"#;
        let actions = OllamaCurationProvider::parse_review_response(response, &cluster);
        // Unknown actions are filtered out (None from filter_map)
        assert_eq!(actions.len(), 0);
    }

    #[test]
    fn test_format_cluster_includes_trust_level() {
        let cluster = vec![make_member("id1", 2.5, 0.75)];
        let formatted = OllamaCurationProvider::format_cluster(&cluster);
        assert!(
            formatted.contains("trust: 0.75"),
            "format_cluster should include trust level, got: {}",
            formatted
        );
    }
}

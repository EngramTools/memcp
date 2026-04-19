//! Curation worker — periodic scan, cluster, and act loop.
//!
//! Orchestrates the full curation pipeline: windowed scan -> candidate fetch ->
//! embedding cluster -> provider review -> action execution -> run tracking.

use std::collections::HashSet;
use std::sync::Arc;

use chrono::Utc;
use metrics;

use super::algorithmic::AlgorithmicCurator;
use super::{ClusterMember, CurationAction, CurationError, CurationProvider, CurationResult};
use crate::config::CurationConfig;
use crate::consolidation::similarity::find_similar_memories;
use crate::store::postgres::PostgresMemoryStore;
use crate::store::postgres::SalienceRow;
use crate::store::{CreateMemory, Memory, MemoryStore};

/// Run a single curation pass.
///
/// Windowed scan -> clustering -> review -> action execution -> run tracking.
/// Called by the daemon on schedule and by CLI `memcp curation run`.
pub async fn run_curation(
    store: &Arc<PostgresMemoryStore>,
    config: &CurationConfig,
    llm_provider: Option<&dyn CurationProvider>,
    dry_run: bool,
) -> Result<CurationResult, CurationError> {
    // 1. Determine window
    let window_start = store
        .get_last_successful_curation_time()
        .await
        .map_err(|e| CurationError::Storage(e.to_string()))?;

    let window_end = Utc::now();

    // 2. Fetch candidates
    let candidates = store
        .get_memories_for_curation(window_start, config.max_candidates_per_run)
        .await
        .map_err(|e| CurationError::Storage(e.to_string()))?;

    if candidates.is_empty() {
        return Ok(CurationResult::skipped("No candidates in window"));
    }

    if dry_run {
        // Dry-run: no run record, no side effects
        return execute_curation(store, config, llm_provider, &candidates, "dry-run", true).await;
    }

    // 3. Create run record
    let run_id = store
        .create_curation_run("auto", window_start, window_end)
        .await
        .map_err(|e| CurationError::Storage(e.to_string()))?;

    // Execute with error handling — on failure, mark run as failed
    match execute_curation(store, config, llm_provider, &candidates, &run_id, false).await {
        Ok(result) => Ok(result),
        Err(e) => {
            let _ = store.fail_curation_run(&run_id, &e.to_string()).await;
            Err(e)
        }
    }
}

/// Inner execution logic — separated for error-handling wrapper.
async fn execute_curation(
    store: &Arc<PostgresMemoryStore>,
    config: &CurationConfig,
    llm_provider: Option<&dyn CurationProvider>,
    candidates: &[(Memory, SalienceRow)],
    run_id: &str,
    dry_run: bool,
) -> Result<CurationResult, CurationError> {
    // 4. Sort candidates by priority (P1 first, then P2, then Normal)
    let mut sorted_candidates = candidates.to_vec();
    sorted_candidates.sort_by_key(|(m, _)| priority_score(m));

    // 5. Build clusters via embedding similarity (P1 seeds become cluster nuclei first)
    let clusters = build_clusters(store, &sorted_candidates, config).await?;

    // 6. Choose provider per-cluster based on priority
    let algorithmic = AlgorithmicCurator::new(config.clone());

    // 7. Review clusters and execute actions
    let mut merged_count = 0usize;
    let mut flagged_count = 0usize;
    let mut strengthened_count = 0usize;
    let mut skipped_count = 0usize;
    let mut suspicious_count = 0usize;
    let mut proposed_actions: Vec<CurationAction> = Vec::new();

    for cluster in &clusters {
        // Priority-aware provider selection:
        // P1/P2 clusters (high-priority) get LLM review when available.
        // Normal clusters use algorithmic only (saves LLM budget for risky memories).
        let provider: &dyn CurationProvider = if cluster_has_high_priority(cluster) {
            match llm_provider {
                Some(p) => p,
                None => &algorithmic,
            }
        } else {
            &algorithmic
        };

        let actions = provider.review_cluster(cluster).await.unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Cluster review failed — skipping cluster");
            cluster
                .iter()
                .map(|m| CurationAction::Skip {
                    memory_id: m.id.clone(),
                    reason: format!("Review failed: {}", e),
                })
                .collect()
        });

        for action in actions {
            match action {
                CurationAction::Merge {
                    source_ids,
                    mut synthesized_content,
                } => {
                    if merged_count >= config.max_merges_per_run {
                        tracing::debug!("Merge cap reached, skipping");
                        continue;
                    }

                    // If content is empty (LLM review said merge but didn't synthesize),
                    // ask provider to synthesize (read-only LLM call, safe for dry_run)
                    if synthesized_content.is_empty() {
                        let sources: Vec<_> = cluster
                            .iter()
                            .filter(|m| source_ids.contains(&m.id))
                            .cloned()
                            .collect();
                        if !sources.is_empty() {
                            synthesized_content = provider
                                .synthesize_merge(&sources)
                                .await
                                .unwrap_or_else(|e| {
                                    tracing::warn!(error = %e, "Merge synthesis failed — using concatenation");
                                    sources
                                        .iter()
                                        .map(|s| s.content.clone())
                                        .collect::<Vec<_>>()
                                        .join("\n\n---\n\n")
                                });
                        }
                    }

                    if dry_run {
                        proposed_actions.push(CurationAction::Merge {
                            source_ids,
                            synthesized_content,
                        });
                        merged_count += 1;
                        continue;
                    }

                    // Collect union of tags and highest stability
                    let mut all_tags: Vec<String> = Vec::new();
                    let mut max_stability = 0.0f64;
                    for member in cluster.iter().filter(|m| source_ids.contains(&m.id)) {
                        all_tags.extend(member.tags.clone());
                        if member.stability > max_stability {
                            max_stability = member.stability;
                        }
                    }
                    all_tags.sort();
                    all_tags.dedup();
                    all_tags.push("merged".to_string());

                    // Create merged memory
                    let new_memory = CreateMemory {
                        content: synthesized_content,
                        type_hint: "curated".to_string(),
                        source: "curation".to_string(),
                        tags: Some(all_tags),
                        created_at: None,
                        actor: None,
                        actor_type: "system".to_string(),
                        audience: "global".to_string(),
                        idempotency_key: None,
                        parent_id: None,
                        chunk_index: None,
                        total_chunks: None,
                        event_time: None,
                        event_time_precision: None,
                        project: None,
                        trust_level: None,
                        session_id: None,
                        agent_role: None,
                        write_path: Some("curation_merge".to_string()),
                        knowledge_tier: None,
                        source_ids: None,
                        reply_to_id: None,
                    };

                    match store.store(new_memory).await {
                        Ok(stored) => {
                            // Set stability to max of sources
                            let _ = store
                                .update_memory_stability(&stored.id, max_stability)
                                .await;

                            // Soft-delete originals
                            let _ = store.soft_delete_memories(&source_ids).await;

                            // Record action
                            let details = serde_json::json!({
                                "source_ids": source_ids,
                                "curated_by": provider.model_name(),
                            });
                            let _ = store
                                .record_curation_action(
                                    run_id,
                                    "merge",
                                    &source_ids,
                                    Some(&stored.id),
                                    None,
                                    Some(details),
                                )
                                .await;

                            merged_count += 1;
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "Failed to create merged memory");
                        }
                    }
                }

                CurationAction::FlagStale { memory_id, reason } => {
                    if flagged_count >= config.max_flags_per_run {
                        tracing::debug!("Flag cap reached, skipping");
                        continue;
                    }

                    if dry_run {
                        proposed_actions.push(CurationAction::FlagStale { memory_id, reason });
                        flagged_count += 1;
                        continue;
                    }

                    // Get current stability for undo tracking
                    let original_stability = cluster
                        .iter()
                        .find(|m| m.id == memory_id)
                        .map(|m| m.stability);

                    // Add 'stale' tag
                    let _ = store.add_memory_tag(&memory_id, "stale").await;

                    // Demote stability
                    let _ = store
                        .update_memory_stability(&memory_id, config.stale_stability_target)
                        .await;

                    // Record action
                    let details = serde_json::json!({ "reason": reason });
                    let _ = store
                        .record_curation_action(
                            run_id,
                            "flag_stale",
                            &[memory_id],
                            None,
                            original_stability,
                            Some(details),
                        )
                        .await;

                    flagged_count += 1;
                }

                CurationAction::Strengthen { memory_id, reason } => {
                    if strengthened_count >= config.max_strengthens_per_run {
                        tracing::debug!("Strengthen cap reached, skipping");
                        continue;
                    }

                    if dry_run {
                        proposed_actions.push(CurationAction::Strengthen { memory_id, reason });
                        strengthened_count += 1;
                        continue;
                    }

                    // Get current stability for undo tracking
                    let original_stability = cluster
                        .iter()
                        .find(|m| m.id == memory_id)
                        .map(|m| m.stability);

                    // Reinforce salience ("good" rating maps to 1.5x stability multiplier)
                    let _ = store.reinforce_salience(&memory_id, "good").await;

                    // Add tag
                    let _ = store
                        .add_memory_tag(&memory_id, "curated:strengthened")
                        .await;

                    // Record action
                    let details = serde_json::json!({ "reason": reason });
                    let _ = store
                        .record_curation_action(
                            run_id,
                            "strengthen",
                            &[memory_id],
                            None,
                            original_stability,
                            Some(details),
                        )
                        .await;

                    strengthened_count += 1;
                }

                CurationAction::Suspicious {
                    memory_id,
                    reason,
                    signals,
                } => {
                    if dry_run {
                        proposed_actions.push(CurationAction::Suspicious {
                            memory_id,
                            reason,
                            signals,
                        });
                        suspicious_count += 1;
                        continue;
                    }
                    let _ = store.add_memory_tag(&memory_id, "curation:flagged").await;
                    tracing::warn!(
                        memory_id = %memory_id,
                        reason = %reason,
                        signals = ?signals,
                        "Suspicious memory flagged by curation — trust lowered to 0.1"
                    );
                    let _ = store
                        .update_trust_level(
                            &memory_id,
                            0.1,
                            &format!("quarantined: {} [signals: {}]", reason, signals.join(", ")),
                        )
                        .await;
                    let details = serde_json::json!({ "reason": reason, "signals": signals });
                    let _ = store
                        .record_curation_action(
                            run_id,
                            "suspicious",
                            &[memory_id],
                            None,
                            None,
                            Some(details),
                        )
                        .await;
                    suspicious_count += 1;
                }

                CurationAction::Skip { memory_id, .. } => {
                    if !dry_run {
                        let _ = store.add_memory_tag(&memory_id, "curation:reviewed").await;
                    }
                    skipped_count += 1;
                }
            }
        }
    }

    // 7. Complete run (skip in dry_run — no run record was created)
    if !dry_run {
        store
            .complete_curation_run(
                run_id,
                merged_count as i32,
                flagged_count as i32,
                strengthened_count as i32,
                skipped_count as i32,
            )
            .await
            .map_err(|e| CurationError::Storage(e.to_string()))?;

        metrics::counter!("memcp_curation_runs_total").increment(1);
        metrics::counter!("memcp_curation_merged_total").increment(merged_count as u64);
        metrics::counter!("memcp_curation_flagged_total").increment(flagged_count as u64);
    }

    Ok(CurationResult {
        run_id: run_id.to_string(),
        merged_count,
        flagged_stale_count: flagged_count,
        strengthened_count,
        skipped_count,
        suspicious_count,
        candidates_processed: candidates.len(),
        clusters_found: clusters.len(),
        skipped_reason: None,
        proposed_actions,
    })
}

/// Build clusters of semantically similar memories using embedding similarity.
///
/// Uses greedy clustering with find_similar_memories from the consolidation module.
/// Caps cluster size at max_merge_group_size (default: 5).
async fn build_clusters(
    store: &Arc<PostgresMemoryStore>,
    candidates: &[(Memory, SalienceRow)],
    config: &CurationConfig,
) -> Result<Vec<Vec<ClusterMember>>, CurationError> {
    let mut visited: HashSet<String> = HashSet::new();
    let mut clusters: Vec<Vec<ClusterMember>> = Vec::new();

    for (memory, salience) in candidates {
        if visited.contains(&memory.id) {
            continue;
        }
        visited.insert(memory.id.clone());

        // Find similar memories via embedding similarity
        // Need the embedding vector for this memory to query pgvector
        let embedding = match store.get_memory_embedding(&memory.id).await {
            Ok(Some(emb)) => emb,
            _ => {
                // No embedding — can't cluster, emit as singleton
                clusters.push(vec![to_cluster_member(memory, salience)]);
                continue;
            }
        };

        let similar = find_similar_memories(
            store.pool(),
            &memory.id,
            &embedding,
            config.cluster_similarity_threshold,
            10,
        )
        .await
        .unwrap_or_default();

        let mut cluster = vec![to_cluster_member(memory, salience)];

        for sim in &similar {
            if visited.contains(&sim.memory_id) {
                continue;
            }
            // Find the matching candidate
            if let Some((sim_mem, sim_sal)) = candidates.iter().find(|(m, _)| m.id == sim.memory_id)
            {
                cluster.push(to_cluster_member(sim_mem, sim_sal));
                visited.insert(sim.memory_id.clone());
            }
        }

        // Cap cluster size — split large clusters into groups
        if cluster.len() > config.max_merge_group_size {
            for chunk in cluster.chunks(config.max_merge_group_size) {
                clusters.push(chunk.to_vec());
            }
        } else {
            clusters.push(cluster);
        }
    }

    Ok(clusters)
}

/// Compute priority score for curation ordering.
///
/// P1 (0): Low trust (<= 0.3) + new (< 1 hour) — highest priority, potential poison.
/// P2 (1): Medium trust (0.3–0.7) + new (< 1 hour) — elevated priority.
/// Normal (2): High trust or old memories — standard processing.
fn priority_score(memory: &Memory) -> u8 {
    let age_minutes = (Utc::now() - memory.created_at).num_minutes();
    let is_new = age_minutes < 60;
    if is_new && memory.trust_level <= 0.3 {
        0 // P1
    } else if is_new && memory.trust_level <= 0.7 {
        1 // P2
    } else {
        2 // Normal
    }
}

/// Check if a cluster contains any high-priority (P1/P2) members.
///
/// Used to decide whether to route to LLM provider for deeper review.
fn cluster_has_high_priority(cluster: &[ClusterMember]) -> bool {
    let now = Utc::now();
    cluster.iter().any(|m| {
        let age_minutes = (now - m.created_at).num_minutes();
        age_minutes < 60 && m.trust_level <= 0.7
    })
}

/// Convert a Memory + SalienceRow into a ClusterMember.
fn to_cluster_member(memory: &Memory, salience: &SalienceRow) -> ClusterMember {
    let tags = memory
        .tags
        .as_ref()
        .and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|t| t.as_str().map(|s| s.to_string()))
                    .collect()
            })
        })
        .unwrap_or_default();

    ClusterMember {
        id: memory.id.clone(),
        content: memory.content.clone(),
        type_hint: if memory.type_hint.is_empty() {
            None
        } else {
            Some(memory.type_hint.clone())
        },
        tags,
        created_at: memory.created_at,
        updated_at: memory.updated_at,
        stability: salience.stability,
        reinforcement_count: salience.reinforcement_count,
        last_reinforced_at: salience.last_reinforced_at,
        trust_level: memory.trust_level,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn make_test_memory(id: &str, trust: f32, age_minutes: i64) -> Memory {
        let now = Utc::now();
        Memory {
            id: id.to_string(),
            content: format!("Content for {}", id),
            type_hint: "fact".to_string(),
            source: "test".to_string(),
            tags: None,
            created_at: now - Duration::minutes(age_minutes),
            updated_at: now - Duration::minutes(age_minutes),
            last_accessed_at: None,
            access_count: 0,
            embedding_status: "complete".to_string(),
            extracted_entities: None,
            extracted_facts: None,
            extraction_status: "complete".to_string(),
            is_consolidated_original: false,
            consolidated_into: None,
            actor: None,
            actor_type: "system".to_string(),
            audience: "global".to_string(),
            parent_id: None,
            chunk_index: None,
            total_chunks: None,
            event_time: None,
            event_time_precision: None,
            project: None,
            trust_level: trust,
            session_id: None,
            agent_role: None,
            write_path: None,
            metadata: serde_json::json!({}),
            abstract_text: None,
            overview_text: None,
            abstraction_status: "skipped".to_string(),
            knowledge_tier: "explicit".to_string(),
            source_ids: None,
            reply_to_id: None,
        }
    }

    fn make_test_salience() -> SalienceRow {
        SalienceRow {
            stability: 2.5,
            difficulty: 5.0,
            reinforcement_count: 0,
            last_reinforced_at: None,
        }
    }

    #[test]
    fn test_priority_score_p1_low_trust_new() {
        let mem = make_test_memory("p1", 0.2, 10); // low trust, 10min old
        assert_eq!(priority_score(&mem), 0, "Low trust + new should be P1");
    }

    #[test]
    fn test_priority_score_p2_medium_trust_new() {
        let mem = make_test_memory("p2", 0.5, 10); // medium trust, 10min old
        assert_eq!(priority_score(&mem), 1, "Medium trust + new should be P2");
    }

    #[test]
    fn test_priority_score_normal_high_trust() {
        let mem = make_test_memory("n1", 0.9, 10); // high trust, new
        assert_eq!(priority_score(&mem), 2, "High trust should be Normal");
    }

    #[test]
    fn test_priority_score_normal_old() {
        let mem = make_test_memory("n2", 0.2, 120); // low trust but old (2hr)
        assert_eq!(
            priority_score(&mem),
            2,
            "Old memory should be Normal regardless of trust"
        );
    }

    #[test]
    fn test_candidates_sorted_by_priority() {
        let mut candidates: Vec<(Memory, SalienceRow)> = vec![
            (make_test_memory("normal", 0.9, 10), make_test_salience()),
            (make_test_memory("p1", 0.2, 10), make_test_salience()),
            (make_test_memory("p2", 0.5, 10), make_test_salience()),
        ];
        candidates.sort_by_key(|(m, _)| priority_score(m));
        assert_eq!(candidates[0].0.id, "p1", "P1 should be first");
        assert_eq!(candidates[1].0.id, "p2", "P2 should be second");
        assert_eq!(candidates[2].0.id, "normal", "Normal should be last");
    }
}

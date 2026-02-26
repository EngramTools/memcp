/// GC worker — implements salience-based pruning, TTL expiry, dry-run, and hard purge.

use anyhow::Result;

use crate::config::GcConfig;
use crate::store::postgres::PostgresMemoryStore;

use super::GcResult;

/// Run garbage collection against the memory store.
///
/// Steps:
/// 1. Count live memories. If <= min_memory_floor, skip (return GcResult::skipped).
/// 2. Compute prune_budget = (live_count - min_memory_floor).max(0).
/// 3. Fetch GC candidates (low-salience + old) up to prune_budget.
/// 4. Fetch TTL-expired memories (no budget limit — expired = must go).
/// 5. In dry_run mode: return candidates without deleting.
/// 6. Otherwise: soft-delete candidates + expired, update GC metrics, hard-purge stale rows.
pub async fn run_gc(
    store: &PostgresMemoryStore,
    config: &GcConfig,
    dry_run: bool,
) -> Result<GcResult> {
    // Step 1: Count live memories
    let live_count = store.count_live_memories().await?;

    // Step 2: Check floor
    if live_count <= config.min_memory_floor as i64 {
        return Ok(GcResult::skipped(format!(
            "below floor ({} live memories, floor = {})",
            live_count, config.min_memory_floor
        )));
    }

    // Step 3: Compute budget and fetch candidates
    let prune_budget = (live_count - config.min_memory_floor as i64).max(0) as i64;
    let candidates = store
        .get_gc_candidates(config.salience_threshold, config.min_age_days, prune_budget)
        .await?;

    // Step 4: Fetch TTL-expired memories
    let expired_ids = store.get_expired_memories().await?;

    if dry_run {
        // Return candidates without deleting
        return Ok(GcResult {
            pruned_count: candidates.len(),
            expired_count: expired_ids.len(),
            hard_purged_count: 0,
            skipped_reason: None,
            candidates,
        });
    }

    // Step 6a: Soft-delete salience candidates
    let candidate_ids: Vec<String> = candidates.iter().map(|c| c.id.clone()).collect();
    let pruned_count = if candidate_ids.is_empty() {
        0
    } else {
        store.soft_delete_memories(&candidate_ids).await?
    };

    // Step 6b: Soft-delete TTL-expired memories
    let expired_count = if expired_ids.is_empty() {
        0
    } else {
        store.soft_delete_memories(&expired_ids).await?
    };

    // Step 6c: Update GC metrics in daemon_status
    let total_pruned = (pruned_count + expired_count) as i64;
    if total_pruned > 0 {
        if let Err(e) = store.update_gc_metrics(total_pruned).await {
            tracing::warn!(error = %e, "Failed to update GC metrics");
        }
    }

    // Step 7: Hard purge old soft-deleted memories beyond grace period
    let hard_purged_count = store
        .hard_purge_old_deleted(config.hard_purge_grace_days)
        .await?;

    Ok(GcResult {
        pruned_count,
        expired_count,
        hard_purged_count,
        skipped_reason: None,
        candidates: vec![], // not populated outside dry-run
    })
}


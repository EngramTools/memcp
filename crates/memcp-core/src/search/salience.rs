/// Salience scoring for memory re-ranking
///
/// Salience is a weighted sum of four independent dimensions, each independently
/// min-max normalized across the result set before weighting:
///   1. Recency   — exponential decay from last_updated
///   2. Access    — log-scale access frequency
///   3. Semantic  — cosine similarity from the query embedding (from RRF / vector search)
///   4. Reinforce — FSRS retrievability (standalone formula, no external crate)
///
/// All scoring functions are pure — no I/O, no database writes.
/// Decay is computed at query time only (never written back) — SRCH-05.

use crate::config::SalienceConfig;
use crate::store::Memory;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Debug breakdown of individual dimension scores (populated only when debug_scoring=true).
#[derive(Debug, Clone)]
pub struct ScoreBreakdown {
    pub recency: f64,
    pub access: f64,
    pub semantic: f64,
    pub reinforcement: f64,
}

/// A single memory hit with RRF and salience scores.
///
/// `rrf_score` is populated by the hybrid search fusion step (Plan 03).
/// `salience_score` is populated by `SalienceScorer::rank()`.
#[derive(Debug, Clone)]
pub struct ScoredHit {
    pub memory: Memory,
    /// RRF fusion score from hybrid search (Plan 03 populates)
    pub rrf_score: f64,
    /// Final weighted salience score (populated by rank())
    pub salience_score: f64,
    /// Origin of this result: "hybrid", "bm25_only", or "vector_only"
    pub match_source: String,
    /// Dimension breakdown — only populated when SalienceConfig.debug_scoring is true
    pub breakdown: Option<ScoreBreakdown>,
}

/// Salience scorer that re-ranks a set of hits using configurable dimension weights.
pub struct SalienceScorer<'a> {
    config: &'a SalienceConfig,
}

// ---------------------------------------------------------------------------
// Pure scoring functions
// ---------------------------------------------------------------------------

/// Exponential recency decay.
///
/// Returns a value in (0, 1] — 1.0 for just-updated, approaching 0 for very old.
/// lambda=0.01 gives ~70-day half-life (ln(2)/0.01 ≈ 69.3 days).
pub fn recency_score(days_since_updated: f64, lambda: f64) -> f64 {
    (-lambda * days_since_updated).exp()
}

/// Log-scale access frequency score.
///
/// Returns value in [0, inf) with diminishing returns:
/// count=0 → 0.0, count=1 → 0.693, count=9 → 2.303, count=99 → 4.615.
pub fn access_frequency_score(access_count: i64) -> f64 {
    (1.0 + access_count as f64).ln()
}

/// FSRS retrievability using the standalone power-law formula.
///
/// Formula: R(t, S) = (1 + F * t / S)^C
/// where F = 19/81, C = -0.5 (FSRS constants from fsrs4anki wiki / borretti.me)
///
/// Returns value clamped to [0.0, 1.0].
/// Returns 0.0 if stability <= 0 (guard against invalid state).
pub fn fsrs_retrievability(stability_days: f64, days_elapsed: f64) -> f64 {
    if stability_days <= 0.0 {
        return 0.0;
    }
    const F: f64 = 19.0 / 81.0;
    const C: f64 = -0.5;
    let r = (1.0 + F * days_elapsed / stability_days).powf(C);
    r.clamp(0.0, 1.0)
}

/// Reinforcement score based on FSRS retrievability.
///
/// Returns the raw retrievability value: high = memory is fresh / well-reinforced.
/// (Plan 04's reinforce_memory will use the inverse for boost calculation.)
pub fn reinforcement_score(stability: f64, days_since_reinforced: f64) -> f64 {
    fsrs_retrievability(stability, days_since_reinforced)
}

/// Min-max normalization over a slice of values.
///
/// Edge case: if max == min (including single-element slices), returns vec![1.0; n]
/// so that a single result or all-identical scores are treated as fully salient.
pub fn normalize(values: &[f64]) -> Vec<f64> {
    if values.is_empty() {
        return Vec::new();
    }
    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    if (max - min).abs() < f64::EPSILON {
        return vec![1.0; values.len()];
    }
    values.iter().map(|&v| (v - min) / (max - min)).collect()
}

// ---------------------------------------------------------------------------
// SalienceScorer
// ---------------------------------------------------------------------------

/// FSRS state for a single memory, passed in from storage layer.
pub struct SalienceInput {
    pub stability: f64,
    pub days_since_reinforced: f64,
}

impl<'a> SalienceScorer<'a> {
    pub fn new(config: &'a SalienceConfig) -> Self {
        SalienceScorer { config }
    }

    /// Re-rank hits by salience score (descending).
    ///
    /// Requires `SalienceInput` for each hit in the same order as `hits`.
    /// The caller fetches salience data from postgres.rs and passes it here.
    ///
    /// Steps:
    /// 1. Compute raw scores for each dimension
    /// 2. Normalize each dimension independently via min-max
    /// 3. Weighted sum: salience = w_r*recency + w_a*access + w_s*semantic + w_re*reinforce
    /// 4. Sort hits by salience descending
    pub fn rank(&self, hits: &mut Vec<ScoredHit>, salience_inputs: &[SalienceInput]) {
        if hits.is_empty() {
            return;
        }

        let cfg = self.config;
        let now_days_reference = 0.0_f64; // "now" is 0 days ago
        let _ = now_days_reference; // used implicitly via days_since_*

        // Step 1: Raw scores
        let raw_recency: Vec<f64> = hits
            .iter()
            .map(|h| {
                let days = days_since(h.memory.updated_at);
                recency_score(days, cfg.recency_lambda)
            })
            .collect();

        let raw_access: Vec<f64> = hits
            .iter()
            .map(|h| access_frequency_score(h.memory.access_count))
            .collect();

        // Semantic score comes from rrf_score (already in [0, inf) range from RRF)
        let raw_semantic: Vec<f64> = hits.iter().map(|h| h.rrf_score).collect();

        let raw_reinforce: Vec<f64> = salience_inputs
            .iter()
            .map(|s| reinforcement_score(s.stability, s.days_since_reinforced))
            .collect();

        // Step 2: Normalize each dimension
        let norm_recency = normalize(&raw_recency);
        let norm_access = normalize(&raw_access);
        let norm_semantic = normalize(&raw_semantic);
        let norm_reinforce = normalize(&raw_reinforce);

        // Step 3: Weighted sum and optional breakdown
        let debug = cfg.debug_scoring;
        for (i, hit) in hits.iter_mut().enumerate() {
            let salience = cfg.w_recency * norm_recency[i]
                + cfg.w_access * norm_access[i]
                + cfg.w_semantic * norm_semantic[i]
                + cfg.w_reinforce * norm_reinforce[i];

            hit.salience_score = salience;
            hit.breakdown = if debug {
                Some(ScoreBreakdown {
                    recency: norm_recency[i],
                    access: norm_access[i],
                    semantic: norm_semantic[i],
                    reinforcement: norm_reinforce[i],
                })
            } else {
                None
            };
        }

        // Step 4: Sort by salience descending
        hits.sort_by(|a, b| b.salience_score.partial_cmp(&a.salience_score).unwrap_or(std::cmp::Ordering::Equal));
    }
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

/// Compute days elapsed since a UTC timestamp.
fn days_since(ts: chrono::DateTime<chrono::Utc>) -> f64 {
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(ts);
    (duration.num_seconds() as f64 / 86_400.0).max(0.0)
}


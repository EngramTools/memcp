/// LoCoMo benchmark runner.
///
/// Orchestrates the full pipeline per sample:
///   truncate -> ingest -> search all QA pairs -> score F1 -> checkpoint.
///
/// Key differences from the LongMemEval runner:
/// - **Per-sample isolation**: truncate once per sample, evaluate all QA pairs against
///   the same ingested state. Avoids re-ingesting 300 turns per QA pair.
/// - **F1 scoring**: no LLM judge needed — compute token-level F1 between hypothesis
///   and ground truth directly.
/// - **Adversarial handling**: category 5 questions score binary (1.0 if hypothesis
///   indicates abstention, 0.0 otherwise).
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use indicatif::{ProgressBar, ProgressStyle};
use reqwest::Client;

use crate::benchmark::{BenchmarkConfig, default_configs};
use crate::embedding::pipeline::EmbeddingPipeline;
use crate::embedding::EmbeddingProvider;
use crate::intelligence::query_intelligence::{QueryIntelligenceProvider, RankedCandidate};
use crate::store::postgres::PostgresMemoryStore;

use super::evaluate::{f1_score, generate_locomo_answer};
use super::ingest::ingest_sample;
use super::{
    LoCoMoIngestionMode, LoCoMoQuestionResult, LoCoMoSample, LoCoMoState, save_locomo_checkpoint,
};

/// Abstention phrases checked for adversarial (category 5) questions.
///
/// If the hypothesis contains any of these substrings (case-insensitive), the
/// model correctly abstained and scores 1.0.
const ABSTENTION_PHRASES: &[&str] = &[
    "don't know",
    "do not know",
    "not available",
    "no information",
    "cannot find",
    "can't find",
    "unable to find",
    "not mentioned",
    "not provided",
    "i don't have",
    "i do not have",
    "not in the",
];

/// Score an adversarial question (category 5) with binary abstention check.
fn score_adversarial(hypothesis: &str) -> f64 {
    let lower = hypothesis.to_lowercase();
    if ABSTENTION_PHRASES
        .iter()
        .any(|phrase| lower.contains(phrase))
    {
        1.0
    } else {
        0.0
    }
}

/// Run the LoCoMo benchmark for a single configuration across all samples.
///
/// For each sample:
/// 1. Skip if already completed (resume support)
/// 2. Flush pipeline + truncate all data (clean slate per sample)
/// 3. Ingest sample conversations via ingest_sample()
/// 4. For each QA pair:
///    a. Embed question, hybrid_search with config weights
///    b. Apply QI expansion/reranking if config enables
///    c. Take top 10 memories
///    d. generate_locomo_answer() for hypothesis
///    e. Score: F1 for categories 1-4, binary abstention for category 5
/// 5. Save checkpoint after each sample (atomic per conversation)
///
/// Returns Vec of LoCoMoQuestionResult for all QA pairs processed.
#[allow(clippy::too_many_arguments)]
pub async fn run_locomo_benchmark(
    samples: &[LoCoMoSample],
    config: &BenchmarkConfig,
    ingestion_mode: &LoCoMoIngestionMode,
    store: Arc<PostgresMemoryStore>,
    pipeline: &EmbeddingPipeline,
    embedding_provider: Arc<dyn EmbeddingProvider>,
    openai_api_key: &str,
    checkpoint_path: &std::path::Path,
    resume_state: Option<LoCoMoState>,
    qi_provider: Option<Arc<dyn QueryIntelligenceProvider>>,
) -> Result<Vec<LoCoMoQuestionResult>, anyhow::Error> {
    let client = Client::new();

    let ingestion_mode_str = match ingestion_mode {
        LoCoMoIngestionMode::PerTurn => "per-turn",
        LoCoMoIngestionMode::PerSession => "per-session",
    };

    // Initialize or restore from checkpoint
    let mut state = resume_state.unwrap_or_else(|| LoCoMoState {
        config_name: config.name.clone(),
        ingestion_mode: ingestion_mode_str.to_string(),
        completed_sample_ids: Vec::new(),
        results: Vec::new(),
        started_at: chrono::Utc::now(),
    });

    // O(1) lookup for already-completed samples
    let completed: HashSet<String> = state.completed_sample_ids.iter().cloned().collect();

    // Progress bar at sample level (each sample = one bar step)
    let pb = ProgressBar::new(samples.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{pos}/{len}] {msg} [{elapsed_precise} / {eta_precise}]")
            .unwrap_or_else(|_| ProgressStyle::default_bar()),
    );

    // Advance bar to reflect already-completed samples from resume
    pb.set_position(completed.len() as u64);

    // Search leg k parameters from config weights
    let bm25_k = if config.bm25_weight > 0.0 {
        Some(60.0f64)
    } else {
        None
    };
    let vector_k = if config.vector_weight > 0.0 {
        Some(60.0f64)
    } else {
        None
    };
    let symbolic_k = if config.symbolic_weight > 0.0 {
        Some(40.0f64)
    } else {
        None
    };

    for sample in samples {
        // Skip already-completed samples (resume support)
        if completed.contains(&sample.sample_id) {
            continue;
        }

        pb.set_message(sample.sample_id.clone());

        // Step 1: Clean slate — drain in-flight embeddings, then truncate DB
        pipeline.flush().await;
        store.truncate_all().await?;

        // Step 2: Ingest sample conversations
        ingest_sample(sample, ingestion_mode, &store, pipeline).await?;

        // Step 3: Evaluate all QA pairs against the ingested state
        let mut sample_results: Vec<LoCoMoQuestionResult> = Vec::new();

        for qa in &sample.qa {
            let qa_start = Instant::now();

            // Step 3a: QI expansion (fail-open)
            let mut search_query = qa.question.clone();

            if config.qi_expansion {
                if let Some(ref provider) = qi_provider {
                    let timeout = Duration::from_secs(2);
                    match tokio::time::timeout(timeout, provider.expand(&qa.question)).await {
                        Ok(Ok(expanded)) => {
                            if let Some(best) = expanded.variants.into_iter().next() {
                                tracing::debug!(
                                    sample_id = %sample.sample_id,
                                    original = %qa.question,
                                    expanded = %best,
                                    "QI expansion rewrote query"
                                );
                                search_query = best;
                            }
                        }
                        Ok(Err(e)) => {
                            tracing::warn!(
                                sample_id = %sample.sample_id,
                                error = %e,
                                "QI expansion failed — using original query"
                            );
                        }
                        Err(_) => {
                            tracing::warn!(
                                sample_id = %sample.sample_id,
                                "QI expansion timed out — using original query"
                            );
                        }
                    }
                }
            }

            // Embed query for vector search leg
            let query_embedding = if vector_k.is_some() {
                match embedding_provider.embed(&search_query).await {
                    Ok(vec) => Some(pgvector::Vector::from(vec)),
                    Err(e) => {
                        tracing::warn!(
                            sample_id = %sample.sample_id,
                            error = %e,
                            "Failed to embed question — falling back to BM25-only"
                        );
                        None
                    }
                }
            } else {
                None
            };

            // Step 3b: Hybrid search
            let mut hits = store
                .hybrid_search(
                    &search_query,
                    query_embedding.as_ref(),
                    20,   // fetch 20 candidates
                    None, // no date filters for LoCoMo
                    None,
                    None,   // no tag filters
                    bm25_k,
                    vector_k,
                    symbolic_k,
                    None, // no source filter
                    None, // no audience filter
                    None, // no project filter
                )
                .await?;

            // Step 3c: QI reranking (fail-open)
            if config.qi_reranking {
                if let Some(ref provider) = qi_provider {
                    let candidates: Vec<RankedCandidate> =
                        hits.iter().enumerate().map(|(i, h)| RankedCandidate {
                            id: h.memory.id.to_string(),
                            content: h.memory.content.chars().take(500).collect(),
                            current_rank: i + 1,
                        }).collect();

                    let timeout = Duration::from_secs(3);
                    match tokio::time::timeout(
                        timeout,
                        provider.rerank(&search_query, &candidates),
                    )
                    .await
                    {
                        Ok(Ok(ranked)) => {
                            let rank_map: HashMap<String, usize> = ranked
                                .iter()
                                .map(|r| (r.id.clone(), r.llm_rank))
                                .collect();
                            hits.sort_by_key(|h| {
                                *rank_map
                                    .get(&h.memory.id.to_string())
                                    .unwrap_or(&usize::MAX)
                            });
                        }
                        Ok(Err(e)) => {
                            tracing::warn!(
                                sample_id = %sample.sample_id,
                                error = %e,
                                "QI reranking failed — using original order"
                            );
                        }
                        Err(_) => {
                            tracing::warn!(
                                sample_id = %sample.sample_id,
                                "QI reranking timed out — using original order"
                            );
                        }
                    }
                }
            }

            // Step 3d: Take top 10 memories for answer generation
            let memories: Vec<_> = hits.into_iter().take(10).map(|h| h.memory).collect();

            // Step 3e: Generate hypothesis
            let hypothesis =
                generate_locomo_answer(&client, openai_api_key, &qa.question, &memories).await?;

            // Step 3f: Score
            let category = qa.category_u8();
            let f1 = if category == 5 {
                // Adversarial: binary abstention check
                score_adversarial(&hypothesis)
            } else {
                // Categories 1-4: SQuAD-style F1
                f1_score(&hypothesis, &qa.answer)
            };

            let latency_ms = qa_start.elapsed().as_millis() as u64;

            sample_results.push(LoCoMoQuestionResult {
                sample_id: sample.sample_id.clone(),
                question: qa.question.clone(),
                answer: qa.answer.clone(),
                hypothesis,
                f1,
                category,
                latency_ms,
            });
        }

        // Step 4: Checkpoint after full sample completion (atomic per conversation)
        state.completed_sample_ids.push(sample.sample_id.clone());
        state.results.extend(sample_results);

        save_locomo_checkpoint(&state, checkpoint_path)?;

        pb.inc(1);
    }

    pb.finish_with_message("done");

    Ok(state.results)
}

/// Map config name to a `BenchmarkConfig`. Convenience for CLI dispatch.
pub fn find_config(name: &str) -> Option<BenchmarkConfig> {
    default_configs().into_iter().find(|c| c.name == name)
}

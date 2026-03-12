//! Background abstraction worker.
//!
//! Polls the database every 5 seconds for memories with abstraction_status='pending'
//! and generates L0 abstracts (and optionally L1 overviews) via the configured LLM provider.
//!
//! Fail-open: on LLM failure, marks the memory abstraction_status='failed' so the
//! embedding pipeline can still embed against full content. Memory remains usable.
//!
//! Metrics:
//! - `memcp_abstraction_jobs_total{status}` — counter per outcome (success|error|skipped)
//! - `memcp_abstraction_duration_seconds` — histogram of LLM generation latency

use std::sync::Arc;
use std::time::Duration;

use crate::config::AbstractionConfig;
use crate::pipeline::abstraction::AbstractionProvider;
use crate::store::postgres::PostgresMemoryStore;

/// Run the abstraction background worker loop.
///
/// Sleeps 5 seconds between polls. For each pending memory:
/// - If content length < `config.min_content_length`, marks as 'skipped'
/// - Otherwise generates L0 abstract (and L1 overview if `config.generate_overview`)
/// - On success: stores abstract_text, overview_text, marks 'complete'
/// - On failure: marks 'failed' with a warning (fail-open)
pub async fn run_abstraction_worker(
    store: Arc<PostgresMemoryStore>,
    provider: Arc<dyn AbstractionProvider>,
    config: AbstractionConfig,
) {
    tracing::info!(
        provider = %config.provider,
        model = %provider.model_name(),
        min_content_length = config.min_content_length,
        generate_overview = config.generate_overview,
        "Abstraction worker started"
    );

    let mut interval = tokio::time::interval(Duration::from_secs(5));

    loop {
        interval.tick().await;

        // Fetch a batch of pending memories
        let pending = match store.get_pending_abstractions(50).await {
            Ok(memories) => memories,
            Err(e) => {
                tracing::warn!(error = %e, "Abstraction worker: failed to fetch pending memories");
                continue;
            }
        };

        if pending.is_empty() {
            continue;
        }

        tracing::debug!(count = pending.len(), "Abstraction worker: processing batch");

        for memory in pending {
            // Skip short content — abstraction adds no value for short snippets
            if memory.content.len() < config.min_content_length {
                if let Err(e) = store.update_abstraction_status(&memory.id, "skipped").await {
                    tracing::warn!(
                        memory_id = %memory.id,
                        error = %e,
                        "Abstraction worker: failed to mark memory as skipped"
                    );
                }
                metrics::counter!(
                    "memcp_abstraction_jobs_total",
                    "status" => "skipped"
                )
                .increment(1);
                continue;
            }

            // Generate L0 abstract
            let start = std::time::Instant::now();
            let abstract_result = provider.generate_abstract(&memory.content).await;
            let duration = start.elapsed().as_secs_f64();

            match abstract_result {
                Err(e) => {
                    tracing::warn!(
                        memory_id = %memory.id,
                        error = %e,
                        "Abstraction worker: L0 generation failed — marking failed (memory still usable)"
                    );
                    let _ = store.update_abstraction_status(&memory.id, "failed").await;
                    metrics::counter!(
                        "memcp_abstraction_jobs_total",
                        "status" => "error"
                    )
                    .increment(1);
                }
                Ok(abstract_text) => {
                    // Optionally generate L1 overview
                    let overview_text: Option<String> = if config.generate_overview {
                        match provider.generate_overview(&memory.content).await {
                            Ok(overview) => Some(overview),
                            Err(e) => {
                                // L1 failure is non-fatal — we still have L0
                                tracing::warn!(
                                    memory_id = %memory.id,
                                    error = %e,
                                    "Abstraction worker: L1 overview generation failed — using L0 only"
                                );
                                None
                            }
                        }
                    } else {
                        None
                    };

                    match store
                        .update_abstraction_fields(
                            &memory.id,
                            &abstract_text,
                            overview_text.as_deref(),
                            "complete",
                        )
                        .await
                    {
                        Ok(()) => {
                            tracing::debug!(
                                memory_id = %memory.id,
                                duration_secs = duration,
                                has_overview = overview_text.is_some(),
                                "Abstraction complete"
                            );
                            metrics::counter!(
                                "memcp_abstraction_jobs_total",
                                "status" => "success"
                            )
                            .increment(1);
                            metrics::histogram!("memcp_abstraction_duration_seconds")
                                .record(duration);
                        }
                        Err(e) => {
                            tracing::warn!(
                                memory_id = %memory.id,
                                error = %e,
                                "Abstraction worker: failed to store abstraction result"
                            );
                            metrics::counter!(
                                "memcp_abstraction_jobs_total",
                                "status" => "error"
                            )
                            .increment(1);
                        }
                    }
                }
            }
        }
    }
}

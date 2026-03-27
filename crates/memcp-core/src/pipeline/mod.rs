//! Pipeline layer — background processing workers.
//!
//! Contains garbage collection, entity extraction, memory consolidation,
//! auto-summarization, auto-store sidecar, content filtering, and curation.
//! All workers are spawned by the daemon (transport/daemon.rs) and
//! process memories from storage/ using intelligence/ providers.

// Pipeline code has ~100+ unwrap() calls in deep processing logic.
// Fixing all is deferred — handler-level safety is enforced by crate-level deny.
#![allow(clippy::unwrap_used)]

pub mod abstraction;
pub mod auto_store;
pub mod chunking;
pub mod consolidation;
pub mod content_filter;
pub mod curation;
pub mod enrichment;
pub mod extraction;
pub mod gc;
pub mod normalization;
pub mod promotion;
pub mod redaction;
pub mod summarization;
pub mod temporal;

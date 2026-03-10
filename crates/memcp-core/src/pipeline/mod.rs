//! Pipeline layer — background processing workers.
//!
//! Contains garbage collection, entity extraction, memory consolidation,
//! auto-summarization, auto-store sidecar, content filtering, and curation.
//! All workers are spawned by the daemon (transport/daemon.rs) and
//! process memories from storage/ using intelligence/ providers.

pub mod auto_store;
pub mod chunking;
pub mod consolidation;
pub mod content_filter;
pub mod curation;
pub mod enrichment;
pub mod extraction;
pub mod gc;
pub mod promotion;
pub mod redaction;
pub mod summarization;
pub mod temporal;

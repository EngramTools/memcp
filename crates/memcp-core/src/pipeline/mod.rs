//! Pipeline layer — background processing workers.
//!
//! Contains garbage collection, entity extraction, memory consolidation,
//! auto-summarization, auto-store sidecar, and content filtering.
//! All workers are spawned by the daemon (transport/daemon.rs) and
//! process memories from storage/ using intelligence/ providers.

pub mod gc;
pub mod extraction;
pub mod consolidation;
pub mod summarization;
pub mod auto_store;
pub mod content_filter;

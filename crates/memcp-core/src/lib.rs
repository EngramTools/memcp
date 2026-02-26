//! memcp — Memory server for AI agents.
//!
//! This crate is organized into four domain layers:
//! - **storage**: Memory persistence (MemoryStore trait, Postgres impl)
//! - **intelligence**: Embedding, search, recall, query intelligence
//! - **pipeline**: Background processing (GC, extraction, consolidation, auto-store, filtering)
//! - **transport**: External interfaces (MCP server, health HTTP, IPC, daemon)
//!
//! Plus top-level modules: config, errors, logging, cli, benchmark.

/// Migrator for `#[sqlx::test]` — runs all migrations in `./migrations/` on ephemeral test DBs.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

// === Domain layers ===
pub mod storage;
pub mod intelligence;
pub mod pipeline;
pub mod transport;

// === Top-level modules ===
pub mod config;
pub mod errors;
pub mod logging;
pub mod cli;
pub mod benchmark;

// === Backward-compatible re-exports ===
// These allow existing `use memcp::store::*`, `use memcp::embedding::*`, etc. to continue working.
// External consumers (binary crate, tests) use these paths. Internal modules use crate:: which
// also resolves through these re-exports.
pub use storage::store;
pub use intelligence::embedding;
pub use intelligence::search;
pub use intelligence::recall;
pub use intelligence::query_intelligence;
pub use pipeline::gc;
pub use pipeline::extraction;
pub use pipeline::consolidation;
pub use pipeline::summarization;
pub use pipeline::auto_store;
pub use pipeline::content_filter;
pub use pipeline::chunking;
pub use transport::server;
pub use transport::health;
pub use transport::ipc;
pub use transport::daemon;

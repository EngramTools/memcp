/// Migrator for `#[sqlx::test]` — runs all migrations in `./migrations/` on ephemeral test DBs.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub mod auto_store;
pub mod benchmark;
pub mod cli;
pub mod config;
pub mod consolidation;
pub mod content_filter;
pub mod daemon;
pub mod embedding;
pub mod errors;
pub mod extraction;
pub mod gc;
pub mod logging;
pub mod query_intelligence;
pub mod search;
pub mod server;
pub mod store;
pub mod summarization;

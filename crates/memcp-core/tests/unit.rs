//! Unit test suite — all tests are pure-logic (no DB, no I/O beyond tempfile).
//! Migrated from inline #[cfg(test)] modules in src/ for clean separation.

#[path = "common/mod.rs"]
mod common;

#[path = "unit/salience.rs"]
mod salience;

#[path = "unit/temporal.rs"]
mod temporal;

#[path = "unit/config.rs"]
mod config;

#[path = "unit/auto_store_parser.rs"]
mod auto_store_parser;

#[path = "unit/auto_store_filter.rs"]
mod auto_store_filter;

#[path = "unit/auto_store_watcher.rs"]
mod auto_store_watcher;

#[path = "unit/auto_store_worker.rs"]
mod auto_store_worker;

#[path = "unit/gc_config.rs"]
mod gc_config;

#[path = "unit/content_filter_regex.rs"]
mod content_filter_regex;

#[path = "unit/content_filter_semantic.rs"]
mod content_filter_semantic;

#[path = "unit/summarization.rs"]
mod summarization;

#[path = "unit/cli.rs"]
mod cli;

#[path = "unit/extraction.rs"]
mod extraction;

#[path = "unit/consolidation.rs"]
mod consolidation;

//! Curation worker — periodic scan, cluster, and act loop.
//!
//! Orchestrates the full curation pipeline: windowed scan → candidate fetch →
//! embedding cluster → provider review → action execution → run tracking.
//! Filled in by Plan 03.

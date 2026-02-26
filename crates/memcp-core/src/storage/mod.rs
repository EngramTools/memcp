//! Storage layer — memory persistence and data types.
//!
//! Contains the MemoryStore trait, Memory/CreateMemory/UpdateMemory types,
//! and the PostgreSQL implementation. All intelligence/ and pipeline/ modules
//! depend on types exported here.

pub mod store;

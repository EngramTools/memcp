//! Transport layer — external interfaces and daemon orchestration.
//!
//! Contains the MCP server (rmcp-based stdio transport), health HTTP
//! endpoints (axum), daemon mode (background worker orchestration),
//! and IPC (Unix socket for daemon<->CLI communication).
//! Wires together storage/, intelligence/, and pipeline/ layers.

pub mod server;
pub mod health;
pub mod ipc;
pub mod daemon;

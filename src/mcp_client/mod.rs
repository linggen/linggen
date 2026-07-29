//! Ling as an MCP **client** — see `doc/mcp-client-spec.md`.
//!
//! The engine has always been able to be *driven by* MCP (`server/mcp.rs`)
//! and never to consume it. Every peer — Claude Code, Codex, Cursor, Cline,
//! Zed — connects to arbitrary MCP servers; this is what lets Linggen do the
//! same.
//!
//! The load-bearing fact: **a model is needed to CHOOSE a tool, not to CALL
//! one.** `tools/call` is a JSON-RPC request, so anything in the engine can
//! make one — including code that runs before the model, which is what lets
//! auto-recall reach a memory server the same way an agent's tool call does.

mod client;
mod config;
mod registry;
mod transport;

pub use client::{McpClient, McpTool};
pub use config::{McpServerConfig, Transport};
pub use registry::{is_mcp_tool, registry, AdvertisedTool, ServerStatus};

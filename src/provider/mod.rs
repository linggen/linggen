//! Provider implementations + model dispatch.
//!
//! Wire-format clients (`anthropic`, `ollama`, `openai`) map the engine's
//! generic [`crate::message::ChatMessage`] to one provider's
//! chat-completions API. `models` defines the shared types they speak
//! (`ChatRequest`, `StreamChunk`, `TokenUsage`, `ToolCallChunk`) plus
//! `ModelManager` — the multi-provider dispatcher. `routing` owns model
//! selection policy + fallback chains. `proxy_provider` is the dispatcher
//! variant used for proxy/relay sessions.

pub mod anthropic;
pub mod claude_auth;
pub mod codex_auth;
pub mod models;
pub mod ollama;
pub mod openai;
pub mod proxy_provider;
pub mod routing;

/// Render a mid-stream body read error with a truthful name: reqwest wraps
/// a `read_timeout` firing between chunks as "error decoding response body",
/// which the transient/fallback classifiers can't recognize. A silent
/// backend must trip the model fallback chain, not read as a decode bug
/// (observed live 2026-07-10: a 35-minute wedge on a stream that never sent
/// a byte after headers).
pub(crate) fn stream_read_error(e: impl std::fmt::Display) -> anyhow::Error {
    let msg = e.to_string();
    if msg.contains("decoding response body") || msg.contains("timed out") {
        anyhow::anyhow!("stream read timed out (backend went silent mid-stream): {msg}")
    } else {
        anyhow::anyhow!("stream error: {msg}")
    }
}

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

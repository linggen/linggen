//! Provider implementations: API clients + provider-specific auth flows.
//!
//! Each submodule is a thin wire-format layer that maps the engine's generic
//! [`crate::message::ChatMessage`] to one provider's chat-completions API.
//! The higher-level routing/fallback logic lives in
//! [`crate::agent_manager::models`].

pub mod anthropic;
pub mod claude_auth;
pub mod codex_auth;
pub mod ollama;
pub mod openai;

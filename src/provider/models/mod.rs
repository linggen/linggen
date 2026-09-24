use crate::config::ModelConfig;
use crate::credentials::{self, Credentials};
use crate::message::ChatMessage;
use crate::provider::anthropic::AnthropicClient;
use crate::provider::codex_auth;
use crate::provider::ollama::OllamaClient;
use crate::provider::openai::OpenAiClient;
use anyhow::Result;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::OnceCell;
use tokio::sync::Semaphore;

mod catalog;
mod health;
mod providers;
mod usage;

pub use catalog::*;
pub use health::*;
pub use usage::*;

use providers::{ChunkStream, Provider, TurnOptions};

/// Keep the model's concurrency permit alive for as long as its stream is
/// being read.
fn hold_permit(stream: ChunkStream, permit: tokio::sync::OwnedSemaphorePermit) -> ChunkStream {
    Box::pin(futures_util::stream::unfold(
        (stream, permit),
        |(mut stream, permit)| async move { stream.next().await.map(|item| (item, (stream, permit))) },
    ))
}

pub struct ModelManager {
    models: HashMap<String, ModelInstance>,
}

struct ModelInstance {
    config: ModelConfig,
    client: Arc<dyn Provider>,
    semaphore: Arc<Semaphore>,
    context_window: OnceCell<Option<usize>>,
    has_vision: OnceCell<bool>,
}

impl ModelManager {
    pub fn new(configs: Vec<ModelConfig>) -> Self {
        let creds = Credentials::load_for(&credentials::credentials_file(), &configs);
        Self::new_with_credentials(configs, &creds)
    }

    /// Verify the model's auth is usable right now. Errors carry an
    /// `AUTH_REQUIRED:` prefix so the chat layer can route them to a
    /// settings/sign-in CTA instead of treating them as generic failures.
    /// Tokens are checked live (not at startup) so we catch both "never
    /// signed in" (file absent) and "expired" the same way.
    fn check_provider_auth(cfg: &ModelConfig) -> Result<()> {
        match cfg.auth_mode.as_deref() {
            Some("chatgpt_oauth") => {
                let tokens = codex_auth::CodexAuthTokens::load(&codex_auth::codex_auth_file());
                if !tokens.is_valid() {
                    let reason = if tokens.access_token.is_none() {
                        "not signed in"
                    } else {
                        "session expired"
                    };
                    anyhow::bail!(
                        "AUTH_REQUIRED: ChatGPT {} for model '{}'. Open Settings → Models → Sign in with ChatGPT.",
                        reason, cfg.id
                    );
                }
            }
            Some("linggen_account") => {
                let Some((token, _)) = crate::account::resolve_token() else {
                    anyhow::bail!(
                        "AUTH_REQUIRED: Not signed in to linggen.dev for model '{}'. Run `ling account login` or sign in from the app.",
                        cfg.id
                    );
                };
                // A key the site has refused is as good as none: the models
                // page and the fallback chain must not read a dead key as
                // signed in.
                if crate::account::token_rejected(&token) {
                    anyhow::bail!(
                        "AUTH_REQUIRED: linggen.dev sign-in expired for model '{}'. Run `ling account login` or sign in from the app.",
                        cfg.id
                    );
                }
            }
            Some("claude_oauth") => {
                let tokens = crate::provider::claude_auth::load().map_err(|_| {
                    anyhow::anyhow!(
                        "AUTH_REQUIRED: Claude not signed in for model '{}'. Run `claude` once to sign in, then retry.",
                        cfg.id
                    )
                })?;
                if !tokens.can_do_inference() {
                    anyhow::bail!(
                        "AUTH_REQUIRED: Claude token for model '{}' is missing the `user:inference` scope. Sign in with `claude` again.",
                        cfg.id
                    );
                }
                if tokens.is_expired() {
                    anyhow::bail!(
                        "AUTH_REQUIRED: Claude session expired for model '{}'. Run `claude` once to refresh.",
                        cfg.id
                    );
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub fn new_with_credentials(configs: Vec<ModelConfig>, creds: &Credentials) -> Self {
        let mut configs = configs;
        inject_linggen_cloud(&mut configs);
        inject_chatgpt_builtin(&mut configs);
        let mut models = HashMap::new();

        // Load ChatGPT OAuth tokens if any model uses chatgpt_oauth
        let codex_tokens = {
            let needs_oauth = configs
                .iter()
                .any(|c| c.auth_mode.as_deref() == Some("chatgpt_oauth"));
            if needs_oauth {
                let tokens = codex_auth::CodexAuthTokens::load(&codex_auth::codex_auth_file());
                if tokens.is_valid() {
                    Some(tokens)
                } else {
                    None
                }
            } else {
                None
            }
        };

        let all_configs = configs.clone();
        for mut cfg in configs {
            // Check if this model uses ChatGPT OAuth
            let is_chatgpt_oauth =
                cfg.auth_mode.as_deref() == Some("chatgpt_oauth") || cfg.provider == "chatgpt";
            // Anthropic with Claude Code OAuth (CC Max subscription).
            let is_claude_oauth =
                cfg.provider == "anthropic" && cfg.auth_mode.as_deref() == Some("claude_oauth");

            if !is_chatgpt_oauth && !is_claude_oauth {
                // Standard: TOML > credentials.json > env var, else a sibling
                // model's key on the same provider + URL.
                cfg.api_key = credentials::resolve_api_key_shared(&cfg, &all_configs, creds);
            }

            let is_linggen_account = cfg.auth_mode.as_deref() == Some("linggen_account");

            let client = match cfg.provider.as_str() {
                _ if is_linggen_account => {
                    Arc::new(OpenAiClient::new_linggen_account(cfg.url.clone()))
                        as Arc<dyn Provider>
                }
                "ollama" => Arc::new(OllamaClient::new(cfg.url.clone(), cfg.api_key.clone())),
                "anthropic" if is_claude_oauth => {
                    Arc::new(AnthropicClient::new_claude_oauth(cfg.url.clone()))
                }
                "anthropic" => Arc::new(AnthropicClient::new(cfg.url.clone(), cfg.api_key.clone())),
                _ if is_chatgpt_oauth => {
                    // ChatGPT OAuth: use subscription tokens
                    if let Some(ref tokens) = codex_tokens {
                        let base_url =
                            if cfg.url.is_empty() || cfg.url == "https://api.openai.com/v1" {
                                codex_auth::CHATGPT_API_BASE.to_string()
                            } else {
                                cfg.url.clone()
                            };
                        Arc::new(OpenAiClient::new_chatgpt_oauth(
                            base_url,
                            tokens.access_token.clone().unwrap_or_default(),
                            tokens.account_id.clone(),
                        ))
                    } else {
                        tracing::warn!(
                            "Model '{}' uses chatgpt_oauth but no valid tokens found. Run `ling auth login`.",
                            cfg.id
                        );
                        Arc::new(OpenAiClient::new(cfg.url.clone(), None))
                    }
                }
                // All other providers (openai, gemini, groq, deepseek, etc.) use OpenAI-compatible API.
                _ => Arc::new(OpenAiClient::new(cfg.url.clone(), cfg.api_key.clone())),
            };
            let semaphore = Arc::new(Semaphore::new(1));
            models.insert(
                cfg.id.clone(),
                ModelInstance {
                    config: cfg,
                    client,
                    semaphore,
                    context_window: OnceCell::new(),
                    has_vision: OnceCell::new(),
                },
            );
        }
        Self { models }
    }

    pub async fn chat_text_stream(
        &self,
        model_id: &str,
        messages: &[ChatMessage],
        effort_override: Option<&str>,
        app: Option<&str>,
    ) -> Result<ChunkStream> {
        let instance = self
            .models
            .get(canonical_model_id(model_id))
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;
        self.chat_text_stream_with_keep_alive(
            model_id,
            messages,
            instance.config.keep_alive.clone(),
            effort_override,
            app,
        )
        .await
    }

    pub async fn chat_text_stream_with_keep_alive(
        &self,
        model_id: &str,
        messages: &[ChatMessage],
        keep_alive: Option<String>,
        effort_override: Option<&str>,
        app: Option<&str>,
    ) -> Result<ChunkStream> {
        let instance = self
            .models
            .get(canonical_model_id(model_id))
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;

        Self::check_provider_auth(&instance.config)?;

        // Note: permit is held for the duration of the stream
        let _permit = instance.semaphore.clone().acquire_owned().await?;

        let stream = instance
            .client
            .text_stream(
                &instance.config,
                messages,
                TurnOptions {
                    keep_alive,
                    effort: effort_override,
                    app,
                },
            )
            .await?;
        Ok(hold_permit(stream, _permit))
    }

    /// Streaming chat with native tool calling support.
    /// Sends tool definitions to the provider and streams back text tokens + tool call chunks.
    pub async fn chat_tool_stream(
        &self,
        model_id: &str,
        messages: &[ChatMessage],
        tools: Vec<serde_json::Value>,
        effort_override: Option<&str>,
        app: Option<&str>,
    ) -> Result<ChunkStream> {
        let instance = self
            .models
            .get(canonical_model_id(model_id))
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;

        Self::check_provider_auth(&instance.config)?;

        let _permit = instance.semaphore.clone().acquire_owned().await?;

        let stream = instance
            .client
            .tool_stream(
                &instance.config,
                messages,
                tools,
                TurnOptions {
                    keep_alive: instance.config.keep_alive.clone(),
                    effort: effort_override,
                    app,
                },
            )
            .await?;
        Ok(hold_permit(stream, _permit))
    }

    pub fn list_models(&self) -> Vec<&ModelConfig> {
        self.models.values().map(|m| &m.config).collect()
    }

    /// Register proxy models from a remote room owner.
    /// Returns the list of registered proxy model IDs for connection tracking.
    pub fn register_proxy_models(
        &mut self,
        proxy_client: Arc<super::proxy_provider::ProxyModelClient>,
        remote_models: Vec<serde_json::Value>,
        owner_name: Option<String>,
    ) -> Vec<String> {
        let mut registered = Vec::new();
        for model_info in remote_models {
            let id = model_info
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let model_name = model_info
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or(&id)
                .to_string();
            let supports_tools = model_info
                .get("supports_tools")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            if id.is_empty() {
                continue;
            }

            let proxy_id = format!("proxy:{id}");
            tracing::info!(
                "Registering proxy model: {proxy_id} (remote: {model_name}, by: {:?})",
                owner_name
            );

            let config = crate::config::ModelConfig {
                id: proxy_id.clone(),
                provider: "proxy".to_string(),
                url: String::new(),
                api_key: None,
                model: model_name,
                context_window: None,
                tags: vec!["proxy".to_string()],
                supports_tools: Some(supports_tools),
                keep_alive: None,
                auth_mode: None,
                reasoning_effort: None,
                provided_by: owner_name.clone(),
                is_builtin: false,
            };

            self.models.insert(
                proxy_id.clone(),
                ModelInstance {
                    config,
                    client: Arc::new(proxy_client.clone()),
                    semaphore: Arc::new(Semaphore::new(1)),
                    context_window: OnceCell::new(),
                    has_vision: OnceCell::new(),
                },
            );
            registered.push(proxy_id);
        }
        registered
    }

    /// Return all registered model IDs (for fallback iteration).
    pub fn model_ids(&self) -> Vec<String> {
        self.models.keys().cloned().collect()
    }

    /// Best-effort cached model context window (num_ctx).
    /// Priority: config override > Ollama /api/show > None.
    pub async fn context_window(&self, model_id: &str) -> Result<Option<usize>> {
        let instance = self
            .models
            .get(canonical_model_id(model_id))
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;

        // Config override takes priority (useful for cloud/remote models).
        if let Some(cw) = instance.config.context_window {
            return Ok(Some(cw));
        }

        // Don't probe a remote /models endpoint without working auth — it
        // would 401 and look like a network hang. Prefer the name-based
        // guess in that case so engine startup keeps moving.
        if Self::check_provider_auth(&instance.config).is_err() {
            let guess = Self::guess_context_window(&instance.config.model);
            return Ok(Some(guess));
        }

        instance
            .client
            .context_window(&instance.config, &instance.context_window)
            .await
    }

    /// Guess context window size from model name for OpenAI-compatible providers.
    /// Returns a conservative default (128K) if the model name is unrecognized.
    pub(super) fn guess_context_window(model_name: &str) -> usize {
        let m = model_name.to_lowercase();
        // Gemini models
        if m.contains("gemini") {
            if m.contains("flash") {
                return 1_048_576;
            } // 1M
            if m.contains("pro") {
                return 1_048_576;
            }
            return 1_048_576;
        }
        // Claude models
        if m.contains("claude") {
            return 200_000;
        }
        // GPT models
        if m.contains("gpt-4o")
            || m.contains("gpt-5")
            || m.contains("gpt-6")
            || m.contains("gpt-4.1")
        {
            return 128_000;
        }
        if m.contains("gpt-4-turbo") || m.contains("gpt-4-1106") {
            return 128_000;
        }
        if m.contains("gpt-4") {
            return 8_192;
        }
        if m.contains("gpt-3.5") {
            return 16_385;
        }
        // DeepSeek
        if m.contains("deepseek") {
            return 128_000;
        }
        // Qwen
        if m.contains("qwen") {
            return 131_072;
        }
        // Conservative default for unknown models
        128_000
    }

    /// Check if a model supports vision (image input).
    /// Ollama: queries /api/show capabilities, cached in OnceCell.
    /// OpenAI-compatible: checks `tags` config for "vision" tag; defaults to true.
    pub async fn has_vision(&self, model_id: &str) -> Result<bool> {
        let instance = self
            .models
            .get(canonical_model_id(model_id))
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;

        instance
            .client
            .has_vision(&instance.config, &instance.has_vision)
            .await
    }

    /// Whether a model can read an image: what the config declares, else what
    /// the family is known for.
    ///
    /// The default matters more than it looks. "OpenAI-compatible" spans models
    /// that see and models that cannot — DeepSeek's V4-Flash returns 400 on an
    /// image and only its separate `-vision` build accepts one — and assuming
    /// yes is how a caller comes to offer a camera that fails at send time,
    /// which is the failure this check exists to prevent. So an unrecognised
    /// model is treated as blind, and says so with the models that are not.
    pub(super) fn declared_or_guessed_vision(tags: &[String], model_name: &str) -> bool {
        if tags.iter().any(|t| t.eq_ignore_ascii_case("no-vision")) {
            return false;
        }
        if tags.iter().any(|t| t.eq_ignore_ascii_case("vision")) {
            return true;
        }
        let m = model_name.to_lowercase();
        // DeepSeek is the case that earned this function: text-only unless the
        // name says otherwise.
        if m.contains("deepseek") {
            return m.contains("vision") || m.contains("-vl");
        }
        m.contains("gemini")
            || m.contains("gpt-4o")
            || m.contains("gpt-4.1")
            || m.contains("gpt-5")
            || m.contains("gpt-6")
            || m.contains("claude")
            || m.contains("vision")
            || m.contains("-vl")
            || m.contains("llava")
    }

    /// Check if a model supports native tool calling.
    /// Uses explicit config if set, otherwise auto-detects based on provider.
    pub fn supports_tools(&self, model_id: &str) -> bool {
        let Some(instance) = self.models.get(canonical_model_id(model_id)) else {
            tracing::warn!(
                "supports_tools: model '{}' not found in configured models (have: {:?}), defaulting to true",
                model_id,
                self.models.keys().collect::<Vec<_>>()
            );
            return true;
        };
        let result = instance.config.supports_tools.unwrap_or(true);
        tracing::debug!("supports_tools: model='{}' → {}", model_id, result);
        result
    }

    /// Check if a model ID exists in the configured models.
    pub fn has_model(&self, model_id: &str) -> bool {
        self.models.contains_key(canonical_model_id(model_id))
    }

    /// Whether [model_id]'s credentials are present right now — the same
    /// pre-flight the stream runs, surfaced so the selection layers (default
    /// chain, fallback, pickers) can pass over a model that cannot answer
    /// instead of learning it from a failed turn.
    pub fn model_auth_ok(&self, model_id: &str) -> bool {
        self.models
            .get(canonical_model_id(model_id))
            .is_some_and(|m| Self::check_provider_auth(&m.config).is_ok())
    }

    /// Return the provider name (`"chatgpt"`, `"openai"`, `"anthropic"`,
    /// `"ollama"`, `"gemini"`, `"deepseek"`, etc.) for a model id. Used by
    /// debug/export surfaces to render tool-defs in the wire shape that
    /// model's provider actually receives — so what you copy from the
    /// system-prompt panel matches what gets sent on the wire.
    pub fn provider_kind(&self, model_id: &str) -> Option<&str> {
        self.models
            .get(canonical_model_id(model_id))
            .map(|m| m.config.provider.as_str())
    }

    /// Return the first OllamaClient found among configured models.
    pub fn first_ollama_client(&self) -> Option<&OllamaClient> {
        self.models.values().find_map(|m| m.client.as_ollama())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vision_is_declared_or_guessed_by_family() {
        let v = |tags: &[&str], model: &str| {
            let tags: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
            ModelManager::declared_or_guessed_vision(&tags, model)
        };

        // The case that earned the function: DeepSeek's chat model 400s on an
        // image, and only its separate vision build takes one. Assuming yes
        // put a camera in front of a model that cannot see.
        assert!(!v(&[], "deepseek-flash"));
        assert!(!v(&[], "deepseek-v4-flash"));
        assert!(!v(&[], "deepseek-v4-pro"));
        assert!(v(&[], "deepseek-v4-flash-vision-exp"));

        // Families that do see keep seeing without anyone editing a config.
        assert!(v(&[], "gemini-3.1-flash-lite-preview"));
        assert!(v(&[], "gpt-6-luna"));
        assert!(v(&[], "claude-sonnet-4-6"));
        assert!(v(&[], "qwen3-vl"));
        assert!(v(&[], "llava"));

        // An unrecognised model is treated as blind: guessing yes fails at
        // send time, guessing no says so and names what can.
        assert!(!v(&[], "some-local-thing"));

        // What the config says always wins, in both directions.
        assert!(v(&["vision"], "some-local-thing"));
        assert!(!v(&["no-vision"], "gpt-6-luna"));
        assert!(!v(&["vision", "no-vision"], "anything"));
    }
}

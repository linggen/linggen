use crate::config::{ModelConfig, ProviderKind, SignIn};
use crate::credentials::{self, Credentials};
use crate::message::ChatMessage;
use crate::provider::anthropic::AnthropicClient;
use crate::provider::kinds::{kind_in, Kind};
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
pub mod chatgpt_catalog;
mod effort;
pub use effort::{Effort, EffortAsk, Resolved};
pub mod families;
pub(crate) mod curated;
pub use curated::{EditionChoice, ModelCatalog};
pub(crate) mod family_catalog;
mod health;
#[cfg(test)]
mod kind_tests;
mod pricing;
mod providers;
pub mod providers_view;
mod readiness;
pub mod spend_cap;
mod usage;

pub use catalog::*;
pub use health::*;
pub use readiness::{NotReady, Readiness, NONE_READY};
pub(crate) use readiness::{cloud_sign_in, rfc3339, usage_limit_line, usage_limit_until};
pub use pricing::*;
pub use usage::*;

use providers::{ChunkStream, Provider, TurnOptions};

/// How many calls a provider's own service answers for one model at the same
/// time. Its limits are the provider's to enforce; ours only keeps a burst of
/// sessions from opening without bound.
const SERVICE_CALLS_AT_ONCE: usize = 6;

/// How many calls ONE model answers at the same time — the only place that
/// decides it. A model run on this machine answers one: a second call would
/// share the same memory and processor and both would crawl. A room's shared
/// model is its owner's to pace, so it stays at one here. A provider's service
/// answers several, so sessions on the same model do not wait for each other
/// (six jobs on one model used to take turns, one reply at a time).
fn calls_at_once(kind: ProviderKind, url: &str) -> usize {
    match kind {
        ProviderKind::Ollama | ProviderKind::Proxy => 1,
        ProviderKind::OpenAiCompatible if on_this_machine(url) => 1,
        ProviderKind::Anthropic | ProviderKind::ChatGpt | ProviderKind::OpenAiCompatible => {
            SERVICE_CALLS_AT_ONCE
        }
    }
}

/// An address on this machine: a model served here, not a provider's service.
fn on_this_machine(url: &str) -> bool {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let authority = rest.split('/').next().unwrap_or("");
    if authority.starts_with("[::1]") {
        return true;
    }
    let host = authority.split(':').next().unwrap_or("");
    matches!(host, "localhost" | "127.0.0.1" | "0.0.0.0") || host.ends_with(".localhost")
}

/// The permits of one model, from [`calls_at_once`].
fn call_slots(kind: ProviderKind, url: &str) -> Arc<Semaphore> {
    Arc::new(Semaphore::new(calls_at_once(kind, url)))
}

/// Keep the model's concurrency permit alive for as long as its stream is
/// being read.
fn hold_permit(stream: ChunkStream, permit: tokio::sync::OwnedSemaphorePermit) -> ChunkStream {
    Box::pin(futures_util::stream::unfold(
        (stream, permit),
        |(mut stream, permit)| async move { stream.next().await.map(|item| (item, (stream, permit))) },
    ))
}

/// The client a ChatGPT-signed-in model speaks through. It reads the tokens
/// when it sends, never here: signing in or out after the manager was built
/// is seen on the next request (signed out, the pre-flight refuses first).
fn chatgpt_client(cfg: &ModelConfig) -> OpenAiClient {
    let base_url = if cfg.url.is_empty() || cfg.url == "https://api.openai.com/v1" {
        codex_auth::CHATGPT_API_BASE.to_string()
    } else {
        cfg.url.clone()
    };
    OpenAiClient::new_chatgpt_oauth(base_url, String::new(), None)
}

pub struct ModelManager {
    models: HashMap<String, ModelInstance>,
    /// Registered for exact pins only (older ChatGPT models the backend still
    /// lists): reachable by id, never listed.
    hidden: std::collections::HashSet<String>,
    /// Live-voice rows (`provider::kinds::Kind::LiveVoice`): kept, never
    /// registered as a chat model.
    live_rows: Vec<ModelConfig>,
}

struct ModelInstance {
    config: ModelConfig,
    client: Arc<dyn Provider>,
    semaphore: Arc<Semaphore>,
    context_window: OnceCell<Option<usize>>,
    /// What the backend's Models API says the model can write in one reply.
    output_cap: OnceCell<Option<u32>>,
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
        match cfg.sign_in() {
            SignIn::ChatGpt => {
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
            SignIn::LinggenAccount => {
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
            SignIn::Claude => {
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
            SignIn::Key | SignIn::Unknown => {}
        }
        Ok(())
    }

    pub fn new_with_credentials(configs: Vec<ModelConfig>, creds: &Credentials) -> Self {
        Self::new_with_catalog(configs, creds, &chatgpt_catalog::current())
    }

    /// As [`new_with_credentials`](Self::new_with_credentials), with the
    /// ChatGPT built-ins taken from [catalog] rather than the one in force.
    pub fn new_with_catalog(
        configs: Vec<ModelConfig>,
        creds: &Credentials,
        catalog: &chatgpt_catalog::Snapshot,
    ) -> Self {
        let mut configs = configs;
        inject_linggen_cloud(&mut configs);
        let hidden = inject_chatgpt_builtin(&mut configs, catalog).into_iter().collect();
        let mut models = HashMap::new();

        // The one chokepoint of kinds (`provider::kinds`): only a chat row
        // gets a client and a place in any list. A live-voice row is kept
        // apart, read by `live_rows` alone; a row the engine has no lane for
        // stays in the config and goes nowhere. One catalog capture decides
        // every row.
        let all_configs = configs.clone();
        let facts = curated::current();
        let (mut chat, mut live_rows) = (Vec::new(), Vec::new());
        for c in configs {
            match kind_in(&c, &facts) {
                Kind::Chat => chat.push(c),
                Kind::LiveVoice => live_rows.push(c),
                Kind::NoLane => {}
                Kind::Speak | Kind::Hear | Kind::Draw => unreachable!("not configured rows"),
            }
        }
        let configs = chat;
        for mut cfg in configs {
            // Check if this model uses ChatGPT OAuth
            let sign_in = cfg.sign_in();
            let kind = cfg.provider_kind();
            let is_chatgpt_oauth = sign_in == SignIn::ChatGpt;
            // Anthropic with Claude Code OAuth (CC Max subscription).
            let is_claude_oauth = kind == ProviderKind::Anthropic && sign_in == SignIn::Claude;

            if !is_chatgpt_oauth && !is_claude_oauth {
                // Standard: TOML > credentials.json > env var, else a sibling
                // model's key on the same provider + URL.
                cfg.api_key = credentials::resolve_api_key_shared(&cfg, &all_configs, creds);
            }

            let is_linggen_account = sign_in == SignIn::LinggenAccount;

            let client = match kind {
                _ if is_linggen_account => {
                    Arc::new(OpenAiClient::new_linggen_account(cfg.url.clone()))
                        as Arc<dyn Provider>
                }
                ProviderKind::Ollama => {
                    Arc::new(OllamaClient::new(cfg.url.clone(), cfg.api_key.clone()))
                }
                ProviderKind::Anthropic if is_claude_oauth => {
                    Arc::new(AnthropicClient::new_claude_oauth(cfg.url.clone()))
                }
                ProviderKind::Anthropic => {
                    Arc::new(AnthropicClient::new(cfg.url.clone(), cfg.api_key.clone()))
                }
                _ if is_chatgpt_oauth => Arc::new(chatgpt_client(&cfg)),
                // All other providers (openai, gemini, groq, deepseek, etc.) use OpenAI-compatible API.
                _ => Arc::new(OpenAiClient::new(cfg.url.clone(), cfg.api_key.clone())),
            };
            let semaphore = call_slots(kind, &cfg.url);
            models.insert(
                cfg.id.clone(),
                ModelInstance {
                    config: cfg,
                    client,
                    semaphore,
                    context_window: OnceCell::new(),
                    output_cap: OnceCell::new(),
                    has_vision: OnceCell::new(),
                },
            );
        }
        Self {
            models,
            hidden,
            live_rows,
        }
    }

    /// The configured live-voice rows: in no chat list, no client. Read by
    /// the providers view alone (`provider::models::providers_view`).
    pub fn live_rows(&self) -> &[ModelConfig] {
        &self.live_rows
    }

    /// The one asker of the monthly spending cap (`spend_cap::check`): a call
    /// to a capped provider is refused before it is made, the session kept.
    fn spend_refusal(&self, cfg: &ModelConfig) -> Result<()> {
        spend_cap::check(cfg).map_err(anyhow::Error::from)
    }

    /// Whether `model_id`'s provider is under its cap this month — the
    /// fallback chain asks it of each candidate.
    pub fn under_spend_cap(&self, model_id: &str) -> bool {
        self.config_of(model_id).is_none_or(|cfg| self.spend_refusal(cfg).is_ok())
    }

    pub async fn chat_text_stream(
        &self,
        model_id: &str,
        messages: &[ChatMessage],
        effort: EffortAsk<'_>,
        app: Option<&str>,
    ) -> Result<ChunkStream> {
        let instance = self
            .models
            .get(self.key(model_id).as_ref())
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;
        self.chat_text_stream_with_keep_alive(
            model_id,
            messages,
            instance.config.keep_alive.clone(),
            effort,
            app,
        )
        .await
    }

    pub async fn chat_text_stream_with_keep_alive(
        &self,
        model_id: &str,
        messages: &[ChatMessage],
        keep_alive: Option<String>,
        effort: EffortAsk<'_>,
        app: Option<&str>,
    ) -> Result<ChunkStream> {
        let instance = self
            .models
            .get(self.key(model_id).as_ref())
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;

        Self::check_provider_auth(&instance.config)?;
        self.spend_refusal(&instance.config)?;

        // Note: permit is held for the duration of the stream
        let _permit = instance.semaphore.clone().acquire_owned().await?;
        let max_output = instance
            .client
            .max_output_tokens(&instance.config, &instance.output_cap, false)
            .await;

        let stream = instance
            .client
            .text_stream(
                &instance.config,
                messages,
                TurnOptions {
                    keep_alive,
                    effort: effort::resolve(&instance.config, &effort).effort,
                    app,
                    max_output,
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
        effort: EffortAsk<'_>,
        app: Option<&str>,
    ) -> Result<ChunkStream> {
        let instance = self
            .models
            .get(self.key(model_id).as_ref())
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;

        Self::check_provider_auth(&instance.config)?;
        self.spend_refusal(&instance.config)?;

        let _permit = instance.semaphore.clone().acquire_owned().await?;
        let max_output = instance
            .client
            .max_output_tokens(&instance.config, &instance.output_cap, true)
            .await;

        let stream = instance
            .client
            .tool_stream(
                &instance.config,
                messages,
                tools,
                TurnOptions {
                    keep_alive: instance.config.keep_alive.clone(),
                    effort: effort::resolve(&instance.config, &effort).effort,
                    app,
                    max_output,
                },
            )
            .await?;
        Ok(hold_permit(stream, _permit))
    }

    pub fn list_models(&self) -> Vec<&ModelConfig> {
        self.models
            .values()
            .filter(|m| !self.hidden.contains(&m.config.id))
            .map(|m| &m.config)
            .collect()
    }

    /// Configured and built-in model rows of the requested kind. Runtime
    /// lanes are reported by the providers view, not the model registry.
    pub fn list_of_kind(&self, kind: Kind) -> Vec<&ModelConfig> {
        match kind {
            Kind::Chat => self.list_models(),
            Kind::LiveVoice => self.live_rows.iter().collect(),
            Kind::Speak | Kind::Hear | Kind::Draw | Kind::NoLane => Vec::new(),
        }
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
                max_output_tokens: None,
                provided_by: owner_name.clone(),
                is_builtin: false,
                family_binding: None,
            price: None,
            };

            self.models.insert(
                proxy_id.clone(),
                ModelInstance {
                    config,
                    client: Arc::new(proxy_client.clone()),
                    semaphore: call_slots(ProviderKind::Proxy, ""),
                    context_window: OnceCell::new(),
                    output_cap: OnceCell::new(),
                    has_vision: OnceCell::new(),
                },
            );
            registered.push(proxy_id);
        }
        registered
    }

    /// Return all registered model IDs (for fallback iteration).
    pub fn model_ids(&self) -> Vec<String> {
        self.models
            .keys()
            .filter(|id| !self.hidden.contains(*id))
            .cloned()
            .collect()
    }

    /// Best-effort cached model context window (num_ctx).
    /// Priority: config override > Ollama /api/show > None.
    pub async fn context_window(&self, model_id: &str) -> Result<Option<usize>> {
        let instance = self
            .models
            .get(self.key(model_id).as_ref())
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;

        // Config override takes priority (useful for cloud/remote models).
        if let Some(cw) = instance.config.context_window {
            return Ok(Some(cw));
        }

        // Don't probe a remote /models endpoint without working auth — it
        // would 401 and look like a network hang. Prefer the name-based
        // guess in that case so engine startup keeps moving.
        if Self::check_provider_auth(&instance.config).is_err() {
            if instance.config.signs_in_with_chatgpt() {
                return Ok(None);
            }
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
        // GPT models, by generation
        if let Some(window) = gpt_context_window(&m) {
            return window;
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
            .get(self.key(model_id).as_ref())
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
            || gpt_sees(&m)
            || m.contains("claude")
            || m.contains("vision")
            || m.contains("-vl")
            || m.contains("llava")
    }

    /// Check if a model supports native tool calling.
    /// Uses explicit config if set, otherwise auto-detects based on provider.
    pub fn supports_tools(&self, model_id: &str) -> bool {
        let Some(instance) = self.models.get(self.key(model_id).as_ref()) else {
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

    /// The registered key a lookup of [id] lands on: the id itself when a
    /// model is configured under it, else its canonical (current) id.
    fn key<'a>(&self, id: &'a str) -> std::borrow::Cow<'a, str> {
        if self.models.contains_key(id) {
            std::borrow::Cow::Borrowed(id)
        } else {
            std::borrow::Cow::Owned(canonical_model_id(id))
        }
    }

    /// The registered id [model_id] resolves to — itself, or the current id
    /// of a retired one — so a caller can store the live id.
    pub fn resolve_id(&self, model_id: &str) -> Option<String> {
        let key = self.key(model_id);
        self.models.contains_key(key.as_ref()).then(|| key.into_owned())
    }

    /// Check if a model ID exists in the configured models.
    pub fn has_model(&self, model_id: &str) -> bool {
        self.models.contains_key(self.key(model_id).as_ref())
    }

    /// Whether [model_id]'s credentials are present right now — the same
    /// pre-flight the stream runs, surfaced so the selection layers (default
    /// chain, fallback, pickers) can pass over a model that cannot answer
    /// instead of learning it from a failed turn.
    pub fn model_auth_ok(&self, model_id: &str) -> bool {
        self.models
            .get(self.key(model_id).as_ref())
            .is_some_and(|m| Self::check_provider_auth(&m.config).is_ok())
    }

    /// Return the provider name (`"chatgpt"`, `"openai"`, `"anthropic"`,
    /// `"ollama"`, `"gemini"`, `"deepseek"`, etc.) for a model id. Used by
    /// debug/export surfaces to render tool-defs in the wire shape that
    /// model's provider actually receives — so what you copy from the
    /// system-prompt panel matches what gets sent on the wire.
    pub fn provider_name(&self, model_id: &str) -> Option<&str> {
        self.models
            .get(self.key(model_id).as_ref())
            .map(|m| m.config.provider_name())
    }

    /// Return the first OllamaClient found among configured models.
    pub fn first_ollama_client(&self) -> Option<&OllamaClient> {
        self.models.values().find_map(|m| m.client.as_ollama())
    }
}

/// The context window an OpenAI model name implies: 128K from GPT-4o and
/// GPT-4.1 on (and GPT-4 Turbo), 8K for the first GPT-4, 16K for GPT-3.5.
fn gpt_context_window(name: &str) -> Option<usize> {
    let (generation, rest) = chatgpt_catalog::openai_generation(name)?;
    let wide_four = ["o", ".1", "-turbo", "-1106"].iter().any(|p| rest.starts_with(p));
    match generation {
        n if n >= 5 => Some(128_000),
        4 if wide_four => Some(128_000),
        4 => Some(8_192),
        3 if rest.starts_with(".5") => Some(16_385),
        _ => None,
    }
}

/// Whether an OpenAI model name sees images: GPT-4o, GPT-4.1, and every
/// generation from 5 on.
fn gpt_sees(name: &str) -> bool {
    match chatgpt_catalog::openai_generation(name) {
        Some((n, _)) if n >= 5 => true,
        Some((4, rest)) => rest.starts_with('o') || rest.starts_with(".1"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    /// A model on this machine answers one call at a time; a provider's
    /// service answers several, so sessions do not queue behind each other.
    #[test]
    fn calls_at_once_is_one_here_and_several_at_a_service() {
        use super::{calls_at_once, SERVICE_CALLS_AT_ONCE};
        use crate::config::ProviderKind as K;
        assert_eq!(calls_at_once(K::Ollama, "http://127.0.0.1:11434"), 1);
        assert_eq!(calls_at_once(K::Proxy, ""), 1);
        assert_eq!(
            calls_at_once(K::OpenAiCompatible, "http://localhost:1234/v1"),
            1
        );
        assert_eq!(
            calls_at_once(K::OpenAiCompatible, "http://[::1]:8080/v1"),
            1
        );
        assert_eq!(
            calls_at_once(K::OpenAiCompatible, "https://api.example.com/v1"),
            SERVICE_CALLS_AT_ONCE
        );
        assert_eq!(
            calls_at_once(K::Anthropic, "https://api.anthropic.com"),
            SERVICE_CALLS_AT_ONCE
        );
        assert_eq!(calls_at_once(K::ChatGpt, ""), SERVICE_CALLS_AT_ONCE);
        assert!(SERVICE_CALLS_AT_ONCE > 1);
    }

    /// The lock: a model's permits are made in ONE place. A second
    /// `Semaphore::new` in this file would be a second rule.
    #[test]
    fn a_models_permits_are_made_in_one_place() {
        let source = include_str!("mod.rs");
        let made = source.matches(concat!("Semaphore::", "new(")).count();
        assert_eq!(made, 1, "make a model's permits with call_slots()");
    }

    use super::*;

    fn byok(id: &str) -> ModelConfig {
        ModelConfig {
            id: id.to_string(),
            provider: "openai".to_string(),
            url: "http://localhost:1".to_string(),
            model: id.to_string(),
            api_key: None,
            keep_alive: None,
            context_window: None,
            tags: vec![],
            supports_tools: Some(true),
            auth_mode: None,
            reasoning_effort: None,
            max_output_tokens: None,
            provided_by: None,
            is_builtin: false,
            family_binding: None,
            price: None,
        }
    }

    /// A backend that records the request line and the Authorization header
    /// of each request, and answers with an empty event stream.
    async fn recording_backend() -> (String, tokio::sync::mpsc::UnboundedReceiver<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 65536];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                let line = head.lines().next().unwrap_or("").to_string();
                let auth = head
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with("authorization:"))
                    .unwrap_or("authorization: none")
                    .to_string();
                let _ = tx.send(format!("{line} | {auth}"));
                let reply = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\ndata: [DONE]\n\n";
                let _ = sock.write_all(reply.as_bytes()).await;
            }
        });
        (format!("http://{addr}"), rx)
    }

    /// A ChatGPT model's client built while signed out sees a sign-in made
    /// afterwards on its next request: the tokens are read when it sends.
    #[tokio::test]
    async fn a_sign_in_after_the_client_was_built_is_used_at_once() {
        let (url, mut seen) = recording_backend().await;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("codex_auth.json");
        let mut cfg = byok("chatgpt-late-sign-in");
        cfg.provider = "chatgpt".into();
        cfg.url = url;
        cfg.auth_mode = Some("chatgpt_oauth".into());
        // Built while signed out (the test home holds no token file).
        let client = chatgpt_client(&cfg).with_auth_file(file.clone());
        codex_auth::CodexAuthTokens {
            access_token: Some("tok-late".into()),
            refresh_token: Some("ref-late".into()),
            account_id: Some("acct-late".into()),
            ..Default::default()
        }
        .save(&file)
        .unwrap();
        let msgs = vec![ChatMessage::new("user", "hi")];
        let _ = client.chat_text_stream("gpt-x", &msgs, None, None).await;
        let request = seen.recv().await.unwrap();
        assert!(request.starts_with("POST /responses "), "{request}");
        assert!(request.ends_with("Bearer tok-late"), "{request}");
    }

    #[test]
    fn retired_ids_resolve_to_their_successor() {
        let m = ModelManager::new_with_credentials(vec![], &Credentials::default());
        // A session or mission still saying an old Sol id lands on 6.2.
        assert!(m.has_model("gpt-6-sol"));
        assert_eq!(m.resolve_id("gpt-6-sol").as_deref(), Some("gpt-6.2-sol"));
        assert_eq!(m.resolve_id("gpt-6.1-sol").as_deref(), Some("gpt-6.2-sol"));
        assert_eq!(m.provider_name("gpt-6-sol"), Some("chatgpt"));
        assert_eq!(
            m.resolve_id("gpt-5.5"),
            Some(chatgpt_catalog::primary_id())
        );
        assert_eq!(
            m.resolve_id("deepseek-v4-flash").as_deref(),
            Some(LINGGEN_CLOUD_MODEL_ID)
        );
        assert_eq!(m.resolve_id("gpt-6.2-sol").as_deref(), Some("gpt-6.2-sol"));
        assert!(m.resolve_id("no-such-model").is_none());
    }

    /// With the backend's list read, an older listed model runs for a pin
    /// that names it, but the pickers still offer exactly the built-ins.
    #[test]
    fn a_listed_older_model_is_reachable_by_pin_and_never_offered() {
        let mut snap = chatgpt_catalog::Snapshot::seed();
        snap.listed = chatgpt_catalog::tests::measured_list();
        let m = ModelManager::new_with_catalog(vec![], &Credentials::default(), &snap);
        let mut offered: Vec<String> = m.list_models().iter().map(|c| c.id.clone()).collect();
        offered.sort();
        let mut want = snap.builtin_ids();
        want.push(LINGGEN_CLOUD_MODEL_ID.to_string());
        want.sort();
        assert_eq!(offered, want);
        let mut ids = m.model_ids();
        ids.sort();
        assert_eq!(ids, want);
        assert_eq!(m.resolve_id("gpt-6.1-sol").as_deref(), Some("gpt-6.1-sol"));
        assert_eq!(m.provider_name("gpt-5.6-luna"), Some("chatgpt"));
        assert_eq!(m.resolve_id("gpt-5.5"), Some(snap.primary_id()));

        // Once the backend stops listing it, a pin moves to its family.
        snap.listed.retain(|l| l.slug != "gpt-6.1-sol");
        let m = ModelManager::new_with_catalog(vec![], &Credentials::default(), &snap);
        assert_eq!(m.resolve_id("gpt-6.1-sol").as_deref(), Some("gpt-6.2-sol"));
    }

    /// With a list read, a person's own model under a listed older id is
    /// still theirs and still offered.
    #[test]
    fn a_user_model_reusing_a_listed_older_id_stays_theirs() {
        let mut snap = chatgpt_catalog::Snapshot::seed();
        snap.listed = chatgpt_catalog::tests::measured_list();
        let m = ModelManager::new_with_catalog(vec![byok("gpt-6-sol")], &Credentials::default(), &snap);
        assert_eq!(m.provider_name("gpt-6-sol"), Some("openai"));
        assert!(m.list_models().iter().any(|c| c.id == "gpt-6-sol"));
    }

    #[test]
    fn a_user_model_reusing_a_retired_id_stays_theirs() {
        let m =
            ModelManager::new_with_credentials(vec![byok("gpt-6-sol")], &Credentials::default());
        assert_eq!(m.resolve_id("gpt-6-sol").as_deref(), Some("gpt-6-sol"));
        assert_eq!(m.provider_name("gpt-6-sol"), Some("openai"));
    }

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

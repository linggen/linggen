use crate::codex_auth;
use crate::config::ModelConfig;
use crate::credentials::{self, Credentials};
use crate::ollama::{ChatMessage, OllamaClient};
use crate::openai::OpenAiClient;
use anyhow::Result;
use futures_util::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::OnceCell;
use tokio::sync::{RwLock, Semaphore};
use tokio::time::Instant;

// ---------------------------------------------------------------------------
// Token usage tracking from API responses
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: Option<usize>,
    pub completion_tokens: Option<usize>,
    pub total_tokens: Option<usize>,
}

/// Items yielded by the streaming chat API.
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// A text token (content or thinking).
    Token(String),
    /// Final usage stats from the API (emitted at end of stream).
    Usage(TokenUsage),
    /// A tool call chunk from native function calling (incremental or complete).
    ToolCall(ToolCallChunk),
}

/// An incremental tool call chunk from the streaming API.
/// For OpenAI: arguments arrive in pieces, keyed by `index`.
/// For Ollama: tool_calls arrive as complete objects per chunk.
#[derive(Debug, Clone)]
pub struct ToolCallChunk {
    /// Index within the tool_calls array (for accumulation).
    pub index: usize,
    /// Tool call ID (usually only present in the first chunk for this index).
    pub id: Option<String>,
    /// Function name (usually only present in the first chunk for this index).
    pub name: Option<String>,
    /// Incremental arguments JSON string fragment.
    pub arguments_delta: Option<String>,
    /// Gemini thought signature (opaque token that must be echoed back).
    pub thought_signature: Option<String>,
}

// ---------------------------------------------------------------------------
// Model health tracking (in-memory, resets on restart)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelHealthStatus {
    Healthy,
    QuotaExhausted,
    Down,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelHealthRecord {
    pub status: ModelHealthStatus,
    #[serde(skip)]
    pub since: Instant,
    pub last_error: Option<String>,
    /// Seconds since the status changed — populated for API responses.
    pub since_secs: Option<u64>,
}

pub struct ModelHealthTracker {
    records: RwLock<HashMap<String, ModelHealthRecord>>,
}

impl ModelHealthTracker {
    pub fn new() -> Self {
        Self {
            records: RwLock::new(HashMap::new()),
        }
    }

    /// Check if a model is available for use.
    /// QuotaExhausted models become available again after 1 hour.
    /// Down models become available again after 5 minutes.
    pub async fn is_available(&self, model_id: &str) -> bool {
        let records = self.records.read().await;
        let Some(record) = records.get(model_id) else {
            return true; // No record = healthy
        };
        match record.status {
            ModelHealthStatus::Healthy => true,
            ModelHealthStatus::QuotaExhausted => record.since.elapsed().as_secs() > 3600,
            ModelHealthStatus::Down => record.since.elapsed().as_secs() > 300,
        }
    }

    pub async fn mark_error(&self, model_id: &str, error_msg: &str) {
        let status = if is_rate_limit_error_str(error_msg) {
            ModelHealthStatus::QuotaExhausted
        } else {
            ModelHealthStatus::Down
        };
        let mut records = self.records.write().await;
        records.insert(
            model_id.to_string(),
            ModelHealthRecord {
                status,
                since: Instant::now(),
                last_error: Some(error_msg.to_string()),
                since_secs: None,
            },
        );
    }

    pub async fn mark_healthy(&self, model_id: &str) {
        let mut records = self.records.write().await;
        records.remove(model_id);
    }

    pub async fn get_all(&self) -> Vec<(String, ModelHealthRecord)> {
        let records = self.records.read().await;
        records
            .iter()
            .map(|(id, rec)| {
                let mut rec = rec.clone();
                rec.since_secs = Some(rec.since.elapsed().as_secs());
                (id.clone(), rec)
            })
            .collect()
    }
}

/// Check if an error message string indicates a rate limit (HTTP 429).
fn is_rate_limit_error_str(msg: &str) -> bool {
    msg.contains("(429)") || msg.to_lowercase().contains("rate limit") || msg.to_lowercase().contains("quota")
}

/// Provider-specific client variant.
enum ProviderClient {
    Ollama(OllamaClient),
    OpenAi(OpenAiClient),
}

pub struct ModelManager {
    models: HashMap<String, ModelInstance>,
    pub health: Arc<ModelHealthTracker>,
}

struct ModelInstance {
    config: ModelConfig,
    client: ProviderClient,
    semaphore: Arc<Semaphore>,
    context_window: OnceCell<Option<usize>>,
    has_vision: OnceCell<bool>,
}

impl ModelManager {
    pub fn new(configs: Vec<ModelConfig>) -> Self {
        let creds = Credentials::load(&credentials::credentials_file());
        Self::new_with_credentials(configs, &creds)
    }

    pub fn new_with_credentials(configs: Vec<ModelConfig>, creds: &Credentials) -> Self {
        let mut models = HashMap::new();

        // Load ChatGPT OAuth tokens if any model uses chatgpt_oauth
        let codex_tokens = {
            let needs_oauth = configs.iter().any(|c| c.auth_mode.as_deref() == Some("chatgpt_oauth"));
            if needs_oauth {
                let tokens = codex_auth::CodexAuthTokens::load(&codex_auth::codex_auth_file());
                if tokens.is_valid() { Some(tokens) } else { None }
            } else {
                None
            }
        };

        for mut cfg in configs {
            // Check if this model uses ChatGPT OAuth
            let is_chatgpt_oauth = cfg.auth_mode.as_deref() == Some("chatgpt_oauth")
                || cfg.provider == "chatgpt";

            if !is_chatgpt_oauth {
                // Standard: resolve API key from TOML > credentials.json > env var
                let effective_key =
                    credentials::resolve_api_key(&cfg.id, cfg.api_key.as_deref(), creds);
                cfg.api_key = effective_key;
            }

            let client = match cfg.provider.as_str() {
                "ollama" => {
                    ProviderClient::Ollama(OllamaClient::new(cfg.url.clone(), cfg.api_key.clone()))
                }
                _ if is_chatgpt_oauth => {
                    // ChatGPT OAuth: use subscription tokens
                    if let Some(ref tokens) = codex_tokens {
                        let base_url = if cfg.url.is_empty() || cfg.url == "https://api.openai.com/v1" {
                            codex_auth::CHATGPT_API_BASE.to_string()
                        } else {
                            cfg.url.clone()
                        };
                        ProviderClient::OpenAi(OpenAiClient::new_chatgpt_oauth(
                            base_url,
                            tokens.access_token.clone().unwrap_or_default(),
                            tokens.account_id.clone(),
                        ))
                    } else {
                        tracing::warn!(
                            "Model '{}' uses chatgpt_oauth but no valid tokens found. Run `ling auth login`.",
                            cfg.id
                        );
                        ProviderClient::OpenAi(OpenAiClient::new(cfg.url.clone(), None))
                    }
                }
                // All other providers (openai, gemini, groq, deepseek, etc.) use OpenAI-compatible API.
                _ => {
                    ProviderClient::OpenAi(OpenAiClient::new(cfg.url.clone(), cfg.api_key.clone()))
                }
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
        Self {
            models,
            health: Arc::new(ModelHealthTracker::new()),
        }
    }

    pub async fn chat_text_stream(
        &self,
        model_id: &str,
        messages: &[ChatMessage],
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let instance = self
            .models
            .get(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;
        self.chat_text_stream_with_keep_alive(
            model_id,
            messages,
            instance.config.keep_alive.clone(),
        )
        .await
    }

    pub async fn chat_text_stream_with_keep_alive(
        &self,
        model_id: &str,
        messages: &[ChatMessage],
        keep_alive: Option<String>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let instance = self
            .models
            .get(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;

        // Note: permit is held for the duration of the stream
        let _permit = instance.semaphore.clone().acquire_owned().await?;

        match &instance.client {
            ProviderClient::Ollama(client) => {
                // Try streaming first; auto-fallback to non-streaming on 503
                // (e.g. Ollama cloud-proxied models that don't support streaming).
                // Retry up to 3 times with backoff for 503 errors (model loading).
                let boxed_stream: Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>> = match client
                    .chat_text_stream_with_keep_alive(
                        &instance.config.model,
                        messages,
                        keep_alive.clone(),
                    )
                    .await
                {
                    Ok(stream) => Box::pin(stream),
                    Err(e) if e.to_string().contains("503") => {
                        tracing::info!(
                            "Streaming returned 503 for model '{}', falling back to non-streaming with retry",
                            instance.config.model
                        );
                        let mut last_err = e;
                        let mut result = None;
                        for attempt in 0..3u32 {
                            if attempt > 0 {
                                let delay = std::time::Duration::from_millis(1000 * (1 << attempt));
                                tracing::info!(
                                    "Retry {}/3 for model '{}' after 503 (waiting {}ms)",
                                    attempt + 1,
                                    instance.config.model,
                                    delay.as_millis()
                                );
                                tokio::time::sleep(delay).await;
                            }
                            match client
                                .chat_text_with_keep_alive(
                                    &instance.config.model,
                                    messages,
                                    keep_alive.clone(),
                                )
                                .await
                            {
                                Ok(msg) => {
                                    result = Some(msg);
                                    break;
                                }
                                Err(e) if e.to_string().contains("503") => {
                                    last_err = e;
                                    continue;
                                }
                                Err(e) => return Err(e),
                            }
                        }
                        match result {
                            Some(msg) => {
                                Box::pin(futures_util::stream::once(async move { Ok(StreamChunk::Token(msg)) }))
                            }
                            None => return Err(last_err),
                        }
                    }
                    Err(e) => return Err(e),
                };
                Ok(Box::pin(futures_util::stream::unfold(
                    (boxed_stream, _permit),
                    |(mut stream, permit)| async move {
                        match stream.next().await {
                            Some(item) => Some((item, (stream, permit))),
                            None => None,
                        }
                    },
                )))
            }
            ProviderClient::OpenAi(client) => {
                let stream = client
                    .chat_text_stream(&instance.config.model, messages)
                    .await?;
                let boxed_stream: Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>> =
                    Box::pin(stream);
                Ok(Box::pin(futures_util::stream::unfold(
                    (boxed_stream, _permit),
                    |(mut stream, permit)| async move {
                        match stream.next().await {
                            Some(item) => Some((item, (stream, permit))),
                            None => None,
                        }
                    },
                )))
            }
        }
    }

    /// Streaming chat with native tool calling support.
    /// Sends tool definitions to the provider and streams back text tokens + tool call chunks.
    pub async fn chat_tool_stream(
        &self,
        model_id: &str,
        messages: &[ChatMessage],
        tools: Vec<serde_json::Value>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let instance = self
            .models
            .get(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;

        let _permit = instance.semaphore.clone().acquire_owned().await?;

        match &instance.client {
            ProviderClient::Ollama(client) => {
                // Retry up to 3 times with backoff for 503 errors (model loading),
                // matching the retry logic in chat_text_stream.
                let mut last_err = None;
                let mut boxed_stream: Option<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> = None;
                for attempt in 0..3u32 {
                    if attempt > 0 {
                        let delay = std::time::Duration::from_millis(1000 * (1 << attempt));
                        tracing::info!(
                            "Retry {}/3 for tool stream model '{}' after 503 (waiting {}ms)",
                            attempt + 1, instance.config.model, delay.as_millis()
                        );
                        tokio::time::sleep(delay).await;
                    }
                    match client
                        .chat_tool_stream_with_keep_alive(
                            &instance.config.model,
                            messages,
                            instance.config.keep_alive.clone(),
                            tools.clone(),
                        )
                        .await
                    {
                        Ok(stream) => {
                            boxed_stream = Some(Box::pin(stream));
                            break;
                        }
                        Err(e) if e.to_string().contains("503") => {
                            tracing::info!(
                                "Tool stream returned 503 for model '{}', retrying...",
                                instance.config.model
                            );
                            last_err = Some(e);
                            continue;
                        }
                        Err(e) => return Err(e),
                    }
                }
                let boxed_stream = match boxed_stream {
                    Some(s) => s,
                    None => return Err(last_err.unwrap_or_else(|| anyhow::anyhow!("503 after retries"))),
                };
                Ok(Box::pin(futures_util::stream::unfold(
                    (boxed_stream, _permit),
                    |(mut stream, permit)| async move {
                        match stream.next().await {
                            Some(item) => Some((item, (stream, permit))),
                            None => None,
                        }
                    },
                )))
            }
            ProviderClient::OpenAi(client) => {
                let stream = client
                    .chat_tool_stream(&instance.config.model, messages, tools)
                    .await?;
                let boxed_stream: Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>> =
                    Box::pin(stream);
                Ok(Box::pin(futures_util::stream::unfold(
                    (boxed_stream, _permit),
                    |(mut stream, permit)| async move {
                        match stream.next().await {
                            Some(item) => Some((item, (stream, permit))),
                            None => None,
                        }
                    },
                )))
            }
        }
    }

    pub fn list_models(&self) -> Vec<&ModelConfig> {
        self.models.values().map(|m| &m.config).collect()
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
            .get(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;

        // Config override takes priority (useful for cloud/remote models).
        if let Some(cw) = instance.config.context_window {
            return Ok(Some(cw));
        }

        match &instance.client {
            ProviderClient::Ollama(client) => {
                let model_name = instance.config.model.clone();
                let client = client.clone();
                let value = instance
                    .context_window
                    .get_or_try_init(|| async move {
                        client.get_model_context_window(&model_name).await
                    })
                    .await;
                Ok(*value?)
            }
            ProviderClient::OpenAi(client) => {
                let model_name = instance.config.model.clone();
                let client = client.clone();
                let value = instance
                    .context_window
                    .get_or_try_init(|| {
                        let model_name = model_name.clone();
                        async move {
                            // Try dynamic API query first, fall back to guess.
                            if let Some(cw) = client.get_context_window(&model_name).await {
                                tracing::debug!("Got context window from API for {model_name}: {cw}");
                                Ok::<_, anyhow::Error>(Some(cw))
                            } else {
                                let guess = Self::guess_context_window(&model_name);
                                tracing::debug!("Using guessed context window for {model_name}: {guess}");
                                Ok(Some(guess))
                            }
                        }
                    })
                    .await;
                Ok(*value?)
            }
        }
    }

    /// Guess context window size from model name for OpenAI-compatible providers.
    /// Returns a conservative default (128K) if the model name is unrecognized.
    fn guess_context_window(model_name: &str) -> usize {
        let m = model_name.to_lowercase();
        // Gemini models
        if m.contains("gemini") {
            if m.contains("flash") { return 1_048_576; } // 1M
            if m.contains("pro") { return 1_048_576; }
            return 1_048_576;
        }
        // Claude models
        if m.contains("claude") {
            return 200_000;
        }
        // GPT models
        if m.contains("gpt-4o") || m.contains("gpt-5") || m.contains("gpt-4.1") {
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
            .get(model_id)
            .ok_or_else(|| anyhow::anyhow!("Model {} not found", model_id))?;

        match &instance.client {
            ProviderClient::Ollama(client) => {
                let model_name = instance.config.model.clone();
                let client = client.clone();
                let value = instance
                    .has_vision
                    .get_or_try_init(|| async move {
                        client.get_model_has_vision(&model_name).await
                    })
                    .await;
                Ok(*value?)
            }
            ProviderClient::OpenAi(_) => {
                // If tags are configured, use them; otherwise default to true.
                if instance.config.tags.is_empty() {
                    Ok(true)
                } else {
                    Ok(instance.config.tags.iter().any(|t| t.eq_ignore_ascii_case("vision")))
                }
            }
        }
    }

    /// Check if a model supports native tool calling.
    /// Uses explicit config if set, otherwise auto-detects based on provider.
    pub fn supports_tools(&self, model_id: &str) -> bool {
        let Some(instance) = self.models.get(model_id) else {
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
        self.models.contains_key(model_id)
    }

    /// Return the first OllamaClient found among configured models.
    pub fn first_ollama_client(&self) -> Option<&OllamaClient> {
        for instance in self.models.values() {
            if let ProviderClient::Ollama(client) = &instance.client {
                return Some(client);
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Error classification for fallback routing
// ---------------------------------------------------------------------------

/// Check if an error indicates a rate limit (HTTP 429).
pub fn is_rate_limit_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string();
    msg.contains("(429)") || msg.to_lowercase().contains("rate limit")
}

/// Check if an error indicates a context/token limit exceeded (HTTP 400 + context keywords).
pub fn is_context_limit_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_lowercase();
    (msg.contains("(400)") || msg.contains("(413)"))
        && (msg.contains("context")
            || msg.contains("token")
            || msg.contains("too long")
            || msg.contains("max_tokens")
            || msg.contains("content_too_large"))
}

/// Returns true if the error is a rate limit or context limit that warrants trying another model.
pub fn is_fallback_worthy_error(err: &anyhow::Error) -> bool {
    is_rate_limit_error(err) || is_context_limit_error(err)
}

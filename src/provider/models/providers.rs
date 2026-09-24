//! The one shape every model backend has to the [`ModelManager`]: stream a
//! text turn, stream a tool turn, say how big its window is and whether it
//! can see. Each client answers in its own way; the manager no longer asks
//! which client it is holding.

use super::{ModelManager, StreamChunk};
use crate::config::ModelConfig;
use crate::message::ChatMessage;
use crate::provider::anthropic::AnthropicClient;
use crate::provider::ollama::OllamaClient;
use crate::provider::openai::OpenAiClient;
use crate::provider::proxy_provider::ProxyModelClient;
use anyhow::Result;
use async_trait::async_trait;
use futures_util::Stream;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::OnceCell;

pub type ChunkStream = Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>;

/// What a turn asks of a backend beyond the messages.
pub struct TurnOptions<'a> {
    pub keep_alive: Option<String>,
    pub effort: Option<&'a str>,
    pub app: Option<&'a str>,
}

#[async_trait]
pub(crate) trait Provider: Send + Sync {
    async fn text_stream(
        &self,
        cfg: &ModelConfig,
        messages: &[ChatMessage],
        opts: TurnOptions<'_>,
    ) -> Result<ChunkStream>;

    async fn tool_stream(
        &self,
        cfg: &ModelConfig,
        messages: &[ChatMessage],
        tools: Vec<serde_json::Value>,
        opts: TurnOptions<'_>,
    ) -> Result<ChunkStream>;

    /// The model's context window; `cache` is the instance's memo, for
    /// backends whose answer does not change.
    async fn context_window(
        &self,
        cfg: &ModelConfig,
        cache: &OnceCell<Option<usize>>,
    ) -> Result<Option<usize>>;

    async fn has_vision(&self, cfg: &ModelConfig, cache: &OnceCell<bool>) -> Result<bool>;

    /// The local Ollama client behind this model, if that is what it is.
    fn as_ollama(&self) -> Option<&OllamaClient> {
        None
    }
}

/// A model still loading answers 503; give it a few seconds.
const LOADING_RETRIES: u32 = 3;

async fn loading_backoff(attempt: u32, what: &str, model: &str) {
    let delay = std::time::Duration::from_millis(1000 * (1 << attempt));
    tracing::info!(
        "Retry {}/{LOADING_RETRIES} for {what} model '{model}' after 503 (waiting {}ms)",
        attempt + 1,
        delay.as_millis()
    );
    tokio::time::sleep(delay).await;
}

#[async_trait]
impl Provider for OllamaClient {
    /// Streaming first; on a 503 (a cloud-proxied model that cannot stream,
    /// or one still loading) fall back to one non-streaming call, retried
    /// with backoff.
    async fn text_stream(
        &self,
        cfg: &ModelConfig,
        messages: &[ChatMessage],
        opts: TurnOptions<'_>,
    ) -> Result<ChunkStream> {
        let first = self
            .chat_text_stream_with_keep_alive(&cfg.model, messages, opts.keep_alive.clone())
            .await;
        let mut last_err = match first {
            Ok(stream) => return Ok(Box::pin(stream)),
            Err(e) if e.to_string().contains("503") => e,
            Err(e) => return Err(e),
        };
        tracing::info!(
            "Streaming returned 503 for model '{}', falling back to non-streaming with retry",
            cfg.model
        );
        for attempt in 0..LOADING_RETRIES {
            if attempt > 0 {
                loading_backoff(attempt, "text", &cfg.model).await;
            }
            match self
                .chat_text_with_keep_alive(&cfg.model, messages, opts.keep_alive.clone())
                .await
            {
                Ok(msg) => {
                    return Ok(Box::pin(futures_util::stream::once(async move {
                        Ok(StreamChunk::Token(msg))
                    })))
                }
                Err(e) if e.to_string().contains("503") => last_err = e,
                Err(e) => return Err(e),
            }
        }
        Err(last_err)
    }

    async fn tool_stream(
        &self,
        cfg: &ModelConfig,
        messages: &[ChatMessage],
        tools: Vec<serde_json::Value>,
        _opts: TurnOptions<'_>,
    ) -> Result<ChunkStream> {
        let mut last_err = None;
        for attempt in 0..LOADING_RETRIES {
            if attempt > 0 {
                loading_backoff(attempt, "tool stream", &cfg.model).await;
            }
            match self
                .chat_tool_stream_with_keep_alive(
                    &cfg.model,
                    messages,
                    cfg.keep_alive.clone(),
                    tools.clone(),
                )
                .await
            {
                Ok(stream) => return Ok(Box::pin(stream)),
                Err(e) if e.to_string().contains("503") => {
                    tracing::info!(
                        "Tool stream returned 503 for model '{}', retrying...",
                        cfg.model
                    );
                    last_err = Some(e);
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("503 after retries")))
    }

    /// Not cached: Ollama's effective num_ctx changes when the user (re)loads
    /// the model with a different size. /api/ps is cheap (local HTTP) and
    /// this is not on a hot path.
    async fn context_window(
        &self,
        cfg: &ModelConfig,
        _cache: &OnceCell<Option<usize>>,
    ) -> Result<Option<usize>> {
        self.get_model_context_window(&cfg.model).await
    }

    async fn has_vision(&self, cfg: &ModelConfig, cache: &OnceCell<bool>) -> Result<bool> {
        let value = cache
            .get_or_try_init(|| self.get_model_has_vision(&cfg.model))
            .await?;
        Ok(*value)
    }

    fn as_ollama(&self) -> Option<&OllamaClient> {
        Some(self)
    }
}

#[async_trait]
impl Provider for OpenAiClient {
    async fn text_stream(
        &self,
        cfg: &ModelConfig,
        messages: &[ChatMessage],
        opts: TurnOptions<'_>,
    ) -> Result<ChunkStream> {
        let effort = opts.effort.or(cfg.reasoning_effort.as_deref());
        let stream = self
            .chat_text_stream(&cfg.model, messages, effort, opts.app)
            .await?;
        Ok(Box::pin(stream))
    }

    async fn tool_stream(
        &self,
        cfg: &ModelConfig,
        messages: &[ChatMessage],
        tools: Vec<serde_json::Value>,
        opts: TurnOptions<'_>,
    ) -> Result<ChunkStream> {
        let effort = opts.effort.or(cfg.reasoning_effort.as_deref());
        let stream = self
            .chat_tool_stream(&cfg.model, messages, tools, effort, opts.app)
            .await?;
        Ok(Box::pin(stream))
    }

    async fn context_window(
        &self,
        cfg: &ModelConfig,
        cache: &OnceCell<Option<usize>>,
    ) -> Result<Option<usize>> {
        let model_name = &cfg.model;
        let value = cache
            .get_or_try_init(|| async move {
                // The ChatGPT-OAuth (Responses API) backend has no /models
                // endpoint — the probe just hangs until the request timeout
                // and stalls the first agent turn on this model. Guess from
                // the name instead.
                if self.uses_responses_api() {
                    return Ok::<_, anyhow::Error>(Some(ModelManager::guess_context_window(
                        model_name,
                    )));
                }
                // Try dynamic API query first, fall back to guess.
                if let Some(cw) = self.get_context_window(model_name).await {
                    tracing::debug!("Got context window from API for {model_name}: {cw}");
                    return Ok(Some(cw));
                }
                let guess = ModelManager::guess_context_window(model_name);
                tracing::debug!("Using guessed context window for {model_name}: {guess}");
                Ok(Some(guess))
            })
            .await?;
        Ok(*value)
    }

    async fn has_vision(&self, cfg: &ModelConfig, _cache: &OnceCell<bool>) -> Result<bool> {
        Ok(ModelManager::declared_or_guessed_vision(
            &cfg.tags, &cfg.model,
        ))
    }
}

#[async_trait]
impl Provider for AnthropicClient {
    async fn text_stream(
        &self,
        cfg: &ModelConfig,
        messages: &[ChatMessage],
        _opts: TurnOptions<'_>,
    ) -> Result<ChunkStream> {
        let stream = self.chat_text_stream(&cfg.model, messages).await?;
        Ok(Box::pin(stream))
    }

    async fn tool_stream(
        &self,
        cfg: &ModelConfig,
        messages: &[ChatMessage],
        tools: Vec<serde_json::Value>,
        _opts: TurnOptions<'_>,
    ) -> Result<ChunkStream> {
        let stream = self.chat_tool_stream(&cfg.model, messages, tools).await?;
        Ok(Box::pin(stream))
    }

    /// Anthropic doesn't expose per-model context window over the API; use
    /// the name-based guess (claude* → 200K).
    async fn context_window(
        &self,
        cfg: &ModelConfig,
        cache: &OnceCell<Option<usize>>,
    ) -> Result<Option<usize>> {
        let value = cache
            .get_or_init(|| async { Some(ModelManager::guess_context_window(&cfg.model)) })
            .await;
        Ok(*value)
    }

    /// All Claude 3.5+ / 4.x models support vision. Respect an explicit
    /// opt-out via `tags = ["no-vision"]` if configured.
    async fn has_vision(&self, cfg: &ModelConfig, _cache: &OnceCell<bool>) -> Result<bool> {
        Ok(!cfg.tags.iter().any(|t| t.eq_ignore_ascii_case("no-vision")))
    }
}

#[async_trait]
impl Provider for Arc<ProxyModelClient> {
    async fn text_stream(
        &self,
        cfg: &ModelConfig,
        messages: &[ChatMessage],
        _opts: TurnOptions<'_>,
    ) -> Result<ChunkStream> {
        self.inference_stream(&cfg.model, messages, None).await
    }

    async fn tool_stream(
        &self,
        cfg: &ModelConfig,
        messages: &[ChatMessage],
        tools: Vec<serde_json::Value>,
        _opts: TurnOptions<'_>,
    ) -> Result<ChunkStream> {
        self.inference_stream(&cfg.model, messages, Some(tools))
            .await
    }

    /// Proxy models: the config's window or a conservative default.
    async fn context_window(
        &self,
        cfg: &ModelConfig,
        _cache: &OnceCell<Option<usize>>,
    ) -> Result<Option<usize>> {
        Ok(cfg.context_window.or(Some(128000)))
    }

    /// The cloud model Linggen supplies is text-only today. It is read the
    /// same way as any other, so the day a vision model is served here it is
    /// a tag or a name rather than a code change.
    async fn has_vision(&self, cfg: &ModelConfig, _cache: &OnceCell<bool>) -> Result<bool> {
        Ok(ModelManager::declared_or_guessed_vision(
            &cfg.tags, &cfg.model,
        ))
    }
}

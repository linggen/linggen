//! Proxy model provider — routes inference through a WebRTC data channel
//! to a room owner's linggen server.
//!
//! Protocol (JSON over data channel):
//! - Request: { type: "inference", request_id, model, messages, tools? }
//! - Response chunks: { request_id, chunk: { type: "token"|"usage"|"tool_call", ... } }
//! - Stream end: { request_id, done: true }
//! - Error: { request_id, error: "..." }

use anyhow::Result;
use futures_util::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::{mpsc, Mutex};

use crate::agent_manager::models::{StreamChunk, ToolCallChunk, TokenUsage};
use crate::message::ChatMessage;

/// A proxy model client that sends inference requests over a WebRTC data channel.
///
/// Wraps the raw request_tx/response_rx from ProxyConnection with request demuxing.
pub struct ProxyModelClient {
    /// Send messages to the WebRTC data channel.
    request_tx: mpsc::Sender<String>,
    /// Per-request response channels, keyed by request_id.
    pending: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<serde_json::Value>>>>,
}

impl ProxyModelClient {
    /// Create a new proxy model client from a ProxyConnection.
    /// Spawns a background demuxer task that routes responses to per-request channels.
    pub fn new(
        request_tx: mpsc::Sender<String>,
        mut response_rx: mpsc::Receiver<String>,
    ) -> Self {
        let pending: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<serde_json::Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Spawn demuxer: reads all responses and routes to the correct per-request channel
        let pending_clone = pending.clone();
        tokio::spawn(async move {
            while let Some(msg) = response_rx.recv().await {
                let val: serde_json::Value = match serde_json::from_str(&msg) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let rid = val.get("request_id").and_then(|v| v.as_str()).unwrap_or("");
                if rid.is_empty() { continue; }

                let pending = pending_clone.lock().await;
                if let Some(tx) = pending.get(rid) {
                    let _ = tx.send(val);
                }
            }
        });

        Self { request_tx, pending }
    }

    /// Send an inference request and return a stream of StreamChunks.
    pub async fn inference_stream(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tools: Option<Vec<serde_json::Value>>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let request_id = format!("inf-{}-{}", chrono::Utc::now().timestamp_millis(),
            rand::random::<u32>() % 100000);

        // Create per-request response channel
        let (resp_tx, resp_rx) = mpsc::unbounded_channel::<serde_json::Value>();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(request_id.clone(), resp_tx);
        }

        // Build and send the request
        let mut req = serde_json::json!({
            "type": "inference",
            "request_id": request_id,
            "model": model,
            "messages": messages,
        });
        if let Some(tools) = tools {
            req["tools"] = serde_json::json!(tools);
        }

        self.request_tx.send(req.to_string()).await
            .map_err(|_| anyhow::anyhow!("Proxy connection closed"))?;

        let pending = self.pending.clone();
        let rid = request_id.clone();

        Ok(Box::pin(ProxyInferenceStream {
            request_id: rid,
            rx: resp_rx,
            pending,
            done: false,
        }))
    }

    /// List models available on the proxy.
    pub async fn list_models(&self) -> Result<Vec<serde_json::Value>> {
        let request_id = format!("lm-{}", chrono::Utc::now().timestamp_millis());

        let (resp_tx, mut resp_rx) = mpsc::unbounded_channel::<serde_json::Value>();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(request_id.clone(), resp_tx);
        }

        let req = serde_json::json!({
            "type": "list_models",
            "request_id": request_id,
        });
        self.request_tx.send(req.to_string()).await
            .map_err(|_| anyhow::anyhow!("Proxy connection closed"))?;

        // Wait for response (with timeout)
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            resp_rx.recv(),
        ).await;

        // Cleanup
        self.pending.lock().await.remove(&request_id);

        match result {
            Ok(Some(val)) => {
                if let Some(err) = val.get("error").and_then(|v| v.as_str()) {
                    anyhow::bail!("Proxy error: {err}");
                }
                let models = val.pointer("/data/models")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                Ok(models)
            }
            Ok(None) => anyhow::bail!("Proxy connection closed"),
            Err(_) => anyhow::bail!("Timeout waiting for model list"),
        }
    }
}

/// A stream that yields StreamChunks from proxy inference responses.
struct ProxyInferenceStream {
    request_id: String,
    rx: mpsc::UnboundedReceiver<serde_json::Value>,
    pending: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<serde_json::Value>>>>,
    done: bool,
}

impl Drop for ProxyInferenceStream {
    fn drop(&mut self) {
        // Clean up the pending entry — use try_lock to avoid blocking
        let pending = self.pending.clone();
        let rid = self.request_id.clone();
        tokio::spawn(async move {
            pending.lock().await.remove(&rid);
        });
    }
}

impl Stream for ProxyInferenceStream {
    type Item = Result<StreamChunk>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.done {
            return Poll::Ready(None);
        }

        match self.rx.poll_recv(cx) {
            Poll::Ready(Some(val)) => {
                // Check for done
                if val.get("done").and_then(|v| v.as_bool()).unwrap_or(false) {
                    self.done = true;
                    return Poll::Ready(None);
                }

                // Check for error
                if let Some(err) = val.get("error").and_then(|v| v.as_str()) {
                    self.done = true;
                    return Poll::Ready(Some(Err(anyhow::anyhow!("Proxy error: {err}"))));
                }

                // Parse chunk
                if let Some(chunk) = val.get("chunk") {
                    let chunk_type = chunk.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    match chunk_type {
                        "token" => {
                            let text = chunk.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            Poll::Ready(Some(Ok(StreamChunk::Token(text))))
                        }
                        "usage" => {
                            Poll::Ready(Some(Ok(StreamChunk::Usage(TokenUsage {
                                prompt_tokens: chunk.get("prompt_tokens").and_then(|v| v.as_u64()).map(|v| v as usize),
                                completion_tokens: chunk.get("completion_tokens").and_then(|v| v.as_u64()).map(|v| v as usize),
                                total_tokens: chunk.get("total_tokens").and_then(|v| v.as_u64()).map(|v| v as usize),
                            }))))
                        }
                        "tool_call" => {
                            Poll::Ready(Some(Ok(StreamChunk::ToolCall(ToolCallChunk {
                                index: chunk.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                                id: chunk.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()),
                                name: chunk.get("name").and_then(|v| v.as_str()).map(|s| s.to_string()),
                                arguments_delta: chunk.get("arguments_delta").and_then(|v| v.as_str()).map(|s| s.to_string()),
                                thought_signature: chunk.get("thought_signature").and_then(|v| v.as_str()).map(|s| s.to_string()),
                            }))))
                        }
                        _ => {
                            // Skip unknown chunk types
                            cx.waker().wake_by_ref();
                            Poll::Pending
                        }
                    }
                } else {
                    // Unknown message format — skip
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            }
            Poll::Ready(None) => {
                self.done = true;
                // Connection dropped before "done" signal — report as error
                Poll::Ready(Some(Err(anyhow::anyhow!("Proxy connection dropped"))))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

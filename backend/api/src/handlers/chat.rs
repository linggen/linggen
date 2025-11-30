use axum::{
    extract::State,
    http::StatusCode,
    response::{sse::Event, Sse},
    Json,
};
use futures::stream::Stream;
use serde::Deserialize;
use std::{convert::Infallible, sync::Arc};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::index::AppState;

#[derive(Deserialize)]
pub struct ChatRequest {
    pub message: String,
    pub context: Option<String>,
}

/// Chat handler that uses Server-Sent Events (SSE) for streaming responses
pub async fn chat_stream(
    State(_state): State<Arc<AppState>>,
    Json(req): Json<ChatRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)> {
    // Get LLM instance
    let llm = linggen_llm::LLMSingleton::get().await;

    if let Some(llm) = llm {
        // Create a channel for streaming tokens
        let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(100);

        // Construct system prompt based on context
        let system_prompt = if let Some(ctx) = &req.context {
            format!(
                "You are a helpful AI assistant. Answer the user's question based on the following context:\n\n{}",
                ctx
            )
        } else {
            "You are a helpful AI assistant.".to_string()
        };

        let message = req.message.clone();

        // Spawn the LLM generation in a blocking thread to avoid blocking the async runtime
        tokio::spawn(async move {
            let mut llm_guard = llm.lock().await;

            // Use a separate channel for the blocking callback
            let (token_tx, mut token_rx) = mpsc::channel::<String>(100);

            // Clone tx for the error case
            let tx_for_error = tx.clone();

            // Spawn a task to forward tokens from the sync callback channel to the SSE channel
            let forward_task = tokio::spawn(async move {
                while let Some(token) = token_rx.recv().await {
                    if tx.send(Ok(Event::default().data(token))).await.is_err() {
                        break; // Client disconnected
                    }
                }
            });

            // Run the generation - the callback will use try_send which is non-blocking
            let result = llm_guard
                .generate_stream(&system_prompt, &message, 1024, |token| {
                    // Use try_send which is non-blocking and doesn't require async
                    match token_tx.try_send(token) {
                        Ok(_) => true,
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            // Channel is full, but we can continue (token will be dropped)
                            // In practice with channel size 100, this shouldn't happen often
                            true
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            // Receiver dropped (client disconnected), stop generation
                            false
                        }
                    }
                })
                .await;

            // Drop the token sender to signal completion
            drop(token_tx);

            // Wait for forwarding to complete
            let _ = forward_task.await;

            if let Err(e) = result {
                let _ = tx_for_error
                    .send(Ok(Event::default().event("error").data(e.to_string())))
                    .await;
            }
        });

        let stream = ReceiverStream::new(rx);
        Ok(Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
    } else {
        Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "LLM is not available. Please enable it in Settings and wait for it to initialize.".to_string(),
        ))
    }
}

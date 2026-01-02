use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
};
use futures::stream::{self, Stream};
use std::{convert::Infallible, sync::Arc, time::Duration};

use super::index::AppState;

pub async fn events_handler(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.broadcast_tx.subscribe();

    let stream = stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Ok(msg) => {
                let event = Event::default().data(msg.to_string());
                Some((Ok(event), rx))
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                // If we lagged, just continue to the next message
                // (The client missed some events but we keep the stream alive)
                let event = Event::default().data("{\"event\": \"lagged\"}");
                Some((Ok(event), rx))
            }
            Err(_) => None, // Channel closed
        }
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

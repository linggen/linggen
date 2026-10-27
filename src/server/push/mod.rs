//! Phone push — the doorbell and the sealed note (doc/phone-push-spec.md).
//!
//! A skill declares the lines it may have Yinyue tell a closed phone; when
//! one is raised and the phone has not told it by the time it is due, the
//! Mac has her write it, seals it with that phone's own key and rings APNs
//! through linggen.dev's relay. The engine names no app: it carries each
//! skill's declaration, its facts and its prompt.

mod api;
mod compose;
mod raise;
mod relay;
mod seal;
mod store;
mod tick;
mod timing;

pub(crate) use api::{delete_device, get_status, post_device, post_notice, post_told};

use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;

use crate::engine::events::{NotificationPayload, ServerEvent};
use crate::server::ServerState;

/// The tick, and the ear on mission ends that raises declared lines.
pub(crate) fn spawn(state: &Arc<ServerState>) {
    tokio::spawn(tick::run(state.clone()));
    tokio::spawn(listen(state.clone()));
}

async fn listen(state: Arc<ServerState>) {
    let mut rx = state.events_tx.subscribe();
    loop {
        match rx.recv().await {
            Ok(ServerEvent::Notification(NotificationPayload::MissionCompleted {
                mission_id,
                status,
                ..
            })) if status == "completed" => {
                let state = state.clone();
                tokio::spawn(async move { raise::after_mission(&state, &mission_id).await });
            }
            Ok(_) => {}
            Err(RecvError::Lagged(n)) => tracing::warn!("[push] lagged; skipped {n} events"),
            Err(RecvError::Closed) => break,
        }
    }
}

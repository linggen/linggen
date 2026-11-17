//! `LINGGEN_EVENT_LOG=<file>`: every event a surface would read, appended as
//! one JSON line each — what the system tests read the live stream from
//! (doc/test-design.md), with the stamp it was sent with (`kept_to`, when
//! kept). Off unless the variable is set.

use super::ServerState;
use std::io::Write;
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;

pub(super) fn spawn(state: &Arc<ServerState>) {
    let Some(path) = std::env::var_os("LINGGEN_EVENT_LOG") else {
        return;
    };
    let mut rx = state.events_tx.listen();
    tokio::spawn(async move {
        let mut seq = 0u64;
        loop {
            let (event, kept_to) = match rx.recv().await {
                Ok(stamped) => (stamped.event, stamped.kept_to),
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return,
            };
            seq += 1;
            let Some(ui) = super::map_server_event_to_ui_message(event, seq) else {
                continue;
            };
            append(&path, &ui, kept_to);
        }
    });
}

fn append(path: &std::ffi::OsStr, ui: &crate::engine::events::UiEvent, kept_to: Option<String>) {
    let Ok(mut line) = serde_json::to_value(ui) else {
        return;
    };
    if let Some(app) = kept_to {
        line["kept_to"] = serde_json::Value::from(app);
    }
    let line = line.to_string();
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path);
    if let Ok(mut f) = file {
        let _ = writeln!(f, "{line}");
    }
}

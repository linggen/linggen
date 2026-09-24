//! Her rolling session: one per day, a fresh segment when the day's thread
//! nears its context limit, seeded with where the last one left off.

use super::*;

/// Pick Yinyue's current rolling session id: one per calendar day, rolling to an
/// extra segment when the active day's live engine nears its context limit. A
/// fresh day (or an unloaded session) always has headroom; continuity across
/// rolls rides shared memory, not the transcript.
pub(super) async fn resolve_current_session(state: &Arc<ServerState>) -> String {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let base = format!("{}-{today}", yinyue_session_prefix());
    let mut seg = 1usize;
    loop {
        let sid = if seg == 1 {
            base.clone()
        } else {
            format!("{base}-{seg}")
        };
        let full = state
            .manager
            .session_context_fraction(&sid)
            .await
            .map(|f| f >= ROLL_TOKEN_FRACTION)
            .unwrap_or(false);
        if !full || seg >= 50 {
            return sid;
        }
        seg += 1;
    }
}

/// Create the session in the store (meta + empty transcript) if it doesn't exist
/// yet, so it persists and lists like any other session.
pub(super) fn ensure_session_exists(state: &Arc<ServerState>, sid: &str, root: &std::path::Path) {
    let exists = state
        .manager
        .global_sessions
        .get_session_meta(sid)
        .map(|m| m.is_some())
        .unwrap_or(false);
    if exists {
        return;
    }
    let label = sid
        .strip_prefix(&format!("{}-", yinyue_session_prefix()))
        .unwrap_or(sid);
    let meta = crate::state_fs::sessions::SessionMeta {
        id: sid.to_string(),
        title: format!("Yinyue · {label}"),
        created_at: crate::util::now_ts_secs(),
        creator: "agent".to_string(),
        cwd: Some(root.to_string_lossy().to_string()),
        title_locked: true,
        ..Default::default()
    };
    if let Err(e) = state.manager.global_sessions.add_session(&meta) {
        tracing::warn!("[yinyue] could not create session {sid}: {e}");
    }
}

/// When this is the very first turn of a freshly rolled session (engine empty
/// AND no persisted history), seed a one-line "Previously" note from the prior
/// Yinyue session's last spoken line so a thread mid-flight doesn't snap across
/// a day/size roll.
pub(super) fn seed_previously_if_fresh(
    state: &Arc<ServerState>,
    engine: &mut crate::engine::AgentEngine,
    session_id: &str,
) {
    if !engine.chat_history.is_empty() {
        return; // engine already warm this process
    }
    let store = &state.manager.global_sessions;
    // Non-empty persisted history → existing session that restore will
    // rehydrate, not a fresh roll. (`unwrap_or(true)` errs toward "don't seed".)
    if store
        .get_chat_history(session_id)
        .map(|h| !h.is_empty())
        .unwrap_or(true)
    {
        return;
    }
    let Some(prev_line) = last_spoken_line(state, session_id) else {
        return;
    };
    engine.chat_history.push(crate::message::ChatMessage::new(
        "system",
        format!("Previously with Han Li: {prev_line}"),
    ));
}

/// The last spoken (assistant) line from the most recent *other* Yinyue session.
pub(super) fn last_spoken_line(state: &Arc<ServerState>, current_sid: &str) -> Option<String> {
    let store = &state.manager.global_sessions;
    let mut sessions = store.list_sessions().ok()?;
    sessions.retain(|m| m.id.starts_with(&yinyue_session_prefix()) && m.id != current_sid);
    sessions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    for meta in sessions {
        let Ok(history) = store.get_chat_history(&meta.id) else {
            continue;
        };
        let line = history.iter().rev().find(|m| {
            !m.is_observation
                && m.from_id != "user"
                && m.from_id != "system"
                && m.from_id != "memory"
                && !m.content.trim().is_empty()
        });
        if let Some(m) = line {
            let line = m.content.trim();
            let capped: String = if line.chars().count() > 240 {
                line.chars().take(240).collect::<String>() + "…"
            } else {
                line.to_string()
            };
            return Some(capped);
        }
    }
    None
}

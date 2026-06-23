//! Yinyue's event-reactive watch loop.
//!
//! Taps the server event bus and, on a few coarse, report-worthy events, wakes
//! the Yinyue agent to decide whether to tell the user. First slice: react only
//! to a *non-Yinyue* mission finishing.
//!
//! The reaction is launched as a plain **agent run** (not a mission), so it (a)
//! never persists a `missions/yinyue-react/` dir that would pollute the mission
//! list, and (b) runs with Yinyue's full `yinyue.md` system prompt rather than a
//! mission body that replaces it.
//!
//! Guards:
//! 1. No self-loop — an agent run does not emit `MissionCompleted`, so a reaction
//!    can't re-trigger this loop. The `yinyue` mission-id check below is kept as
//!    belt-and-suspenders for any future mission-shaped reaction.
//! 2. Cost — match only the coarse event(s); the per-token firehose
//!    (`Token` / `TextSegment` / `ContentBlock*`) falls through the `else` arm at
//!    near-zero cost. The LLM is woken only on a narrow trigger.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;

use super::events::{NotificationPayload, ServerEvent};
use super::state::ServerState;

const YINYUE_AGENT: &str = "yinyue";
/// Yinyue's sessions roll daily (`sess-yinyue-YYYY-MM-DD`) with an extra
/// segment (`…-2`, `…-3`) when a day's thread nears its context limit. One
/// session is active at a time, so turns still serialize through a single
/// engine lock and read as a continuing thread.
const YINYUE_SESSION_PREFIX: &str = "sess-yinyue";
/// Roll to a fresh segment once the live engine crosses this fraction of its
/// soft context limit — she starts clean and leans on memory rather than
/// compacting a long companion transcript.
const ROLL_TOKEN_FRACTION: f32 = 0.7;
/// Cap on how many recent messages stay in Yinyue's live prompt. Her one rolling
/// session would otherwise drag a whole day of turns into every quick reply; the
/// rest lives on disk + in recalled memory. Small = snappy.
const YINYUE_MAX_LIVE_MSGS: usize = 10;
/// The metered Linggen Cloud model — Yinyue's default brain for signed-in
/// (paid/free) users when `pet.model` is "auto". BYOK users keep the engine
/// default unless they pick a model in settings.
const CLOUD_DEFAULT_MODEL: &str = "deepseek-v4-flash";

pub async fn yinyue_watch_loop(state: Arc<ServerState>) {
    let mut rx = state.events_tx.subscribe();
    tracing::info!("[yinyue-watch] started");
    loop {
        match rx.recv().await {
            Ok(event) => handle_event(&state, event),
            Err(RecvError::Lagged(n)) => {
                tracing::warn!("[yinyue-watch] lagged; skipped {n} events");
            }
            Err(RecvError::Closed) => break,
        }
    }
}

/// Cheap, synchronous classifier. Matches the coarse triggers and spawns any
/// async follow-up; every other event returns immediately.
fn handle_event(state: &Arc<ServerState>, event: ServerEvent) {
    let ServerEvent::Notification(payload) = event else {
        return; // firehose + all other events dropped here, near-free
    };
    match payload {
        // A background mission finished — wake Yinyue to decide whether it's worth
        // a word. A real LLM reaction, because she may have something to say.
        NotificationPayload::MissionCompleted {
            mission_id,
            mission_name,
            status,
            ..
        } => {
            // Guard: never react to a Yinyue-shaped mission (belt-and-suspenders).
            if mission_id.starts_with(YINYUE_AGENT) {
                return;
            }
            tracing::info!(
                "[yinyue-watch] mission '{mission_id}' completed ({status}); waking Yinyue"
            );
            let state = state.clone();
            tokio::spawn(async move {
                wake_for_mission(state, &mission_name, &status).await;
            });
        }
        // A run errored — Yinyue surfaces it in her own voice. A DETERMINISTIC
        // line over the speak spine, never an LLM wake: the failure may be the
        // model backend itself, so waking her agent to announce it could fail too
        // (and loop). Guarded against her own failures + rate-limited.
        NotificationPayload::RunFailed { agent_id, .. } => {
            if agent_id == YINYUE_AGENT {
                return; // no self-loop — her own hiccup mustn't beget another
            }
            if !error_announce_allowed() {
                return; // an error storm shouldn't make her repeat herself
            }
            // Gate on the Pet master switch (async read), then surface it.
            let state = state.clone();
            tokio::spawn(async move {
                if !state.manager.get_config_snapshot().await.pet.enabled {
                    return;
                }
                tracing::info!("[yinyue-watch] run by '{agent_id}' failed; Yinyue surfaces it");
                crate::server::api::yinyue::emit_speak(&state, error_line(), Some("sad".to_string()));
            });
        }
    }
}

/// Min seconds between error announcements. Several runs can fail within seconds
/// (a flaky model backend), and she should mention it once, not chant.
const ERROR_ANNOUNCE_COOLDOWN_SECS: u64 = 90;
static LAST_ERROR_ANNOUNCE: AtomicU64 = AtomicU64::new(0);

/// True at most once per cooldown window; stamps the clock when it returns true.
fn error_announce_allowed() -> bool {
    let now = crate::util::now_ts_secs();
    let last = LAST_ERROR_ANNOUNCE.load(Ordering::Relaxed);
    if now.saturating_sub(last) < ERROR_ANNOUNCE_COOLDOWN_SECS {
        return false;
    }
    LAST_ERROR_ANNOUNCE.store(now, Ordering::Relaxed);
    true
}

/// A brief, in-character "something's wrong" — her voice, never the raw error,
/// never jargon. Rotated so repeated failures don't read like a canned alert.
fn error_line() -> String {
    const LINES: [&str; 4] = [
        "I'm sorry — something just went wrong behind the scenes.",
        "Something stumbled just now. I'm sorry to bring it up.",
        "That didn't go through cleanly — my apologies. I'm keeping an eye on it.",
        "Something's off just now, and I wanted you to know.",
    ];
    let idx = (crate::util::now_ts_secs() as usize) % LINES.len();
    LINES[idx].to_string()
}

/// Wake the Yinyue agent to react to a finished background mission. She decides
/// whether it's worth surfacing — replying `SILENT` means say nothing (the
/// never-nag discipline). Anything else is spoken to her surfaces.
async fn wake_for_mission(state: Arc<ServerState>, mission_name: &str, status: &str) {
    let task = format!(
        "You've been woken to react to a background event on the user's machine. \
         The background job \"{mission_name}\" just finished (status: {status}). \
         Decide whether it's worth telling the user. If so, reply with one or two brief \
         sentences in your voice — what happened and anything notable (you may Memory_query, \
         Read, or Grep for context). Your reply will be SPOKEN ALOUD, so write plain prose, \
         no markdown. If it's routine and not worth interrupting them, reply with exactly the \
         single word SILENT and nothing else. Be brief. Never nag."
    );

    let Some(line) = run_yinyue_turn(&state, task).await else {
        return; // run failed or she produced nothing
    };
    if line.eq_ignore_ascii_case("silent") {
        tracing::info!("[yinyue-watch] Yinyue chose silence");
        return;
    }
    let emotion = if status.eq_ignore_ascii_case("completed") || status.to_lowercase().contains("success") {
        "happy"
    } else {
        "neutral"
    };
    tracing::info!("[yinyue-watch] Yinyue speaks ({} chars, {emotion})", line.len());
    crate::server::api::yinyue::emit_speak(&state, line, Some(emotion.to_string()));
}

/// Run one Yinyue turn on her current rolling session and return her final line
/// (trimmed; `None` if she produced no text). The single place that drives the
/// Yinyue agent — used by the event-reactive watch above and by the "talk to
/// her" endpoint (`api::yinyue::chat_handler`).
///
/// She is an ordinary session running the `yinyue` agent: the turn goes through
/// the **shared turn-core** (`chat::run_session_turn`), so she gets the same
/// persistence, restart-reload, auto-recall, capture, and compaction as Ling.
/// The only Yinyue-specific bits are the rolling session id, the spoken output
/// (handled by callers via `emit_speak`), and her narrow tool list (`yinyue.md`).
pub(crate) async fn run_yinyue_turn(state: &Arc<ServerState>, task: String) -> Option<String> {
    let root = crate::util::resolve_path(std::path::Path::new("~/.linggen"));

    // Pet settings (Settings → General → Pet). Disabled → she doesn't run at all.
    let pet = state.manager.get_config_snapshot().await.pet;
    if !pet.enabled {
        return None;
    }

    // Rolling session: one per day, segmented when a day's thread fills up.
    let session_id = resolve_current_session(state).await;
    ensure_session_exists(state, &session_id, &root);

    let agent = match state
        .manager
        .get_or_create_session_agent(&session_id, &root, YINYUE_AGENT)
        .await
    {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("[yinyue] could not create Yinyue agent: {e}");
            return None;
        }
    };

    let run_id = state
        .manager
        .begin_agent_run(
            &root,
            Some(session_id.as_str()),
            YINYUE_AGENT,
            None,
            Some("yinyue".to_string()),
        )
        .await
        .unwrap_or_else(|_| format!("run-{YINYUE_AGENT}-fallback"));

    // Persist the incoming message to the session store so it survives reload
    // and the turn-core's restore sees a complete thread. (The turn core only
    // mirrors it into in-memory history; disk persistence happens here — the
    // same split the Web-UI chat handler uses.)
    crate::server::chat::helpers::persist_message_only(
        &state.manager,
        &root,
        YINYUE_AGENT,
        "user",
        YINYUE_AGENT,
        &task,
        Some(&session_id),
        false,
    )
    .await;

    let spoken = {
        let mut engine = agent.lock().await;
        engine.set_parent_agent(None);
        engine.set_run_id(Some(run_id.clone()));
        // Clear so we read THIS turn's final line — the engine is reused across
        // turns and would otherwise hold the prior one.
        engine.last_assistant_text = None;

        // Tune her memory injection from the Pet settings (default: one
        // high-relevance record at ≥0.8). Set on her own engine's cfg (a
        // per-session clone), so Ling's full-store recall is untouched.
        // Idempotent — safe to set each turn, and picks up live settings edits.
        engine.cfg.memory_recall_count = pet.recall_count.max(1);
        engine.cfg.memory_inject_min_score = Some(pet.recall_min_score);

        // Pick her brain per the Pet model setting (tier-aware default: the
        // metered Linggen Cloud model for signed-in users, the engine default
        // for BYOK). An unavailable id falls back to whatever she's already on.
        if let Some(m) = resolve_pet_model(&pet.model) {
            if engine.model_manager.has_model(&m) {
                engine.model_id = m;
            } else {
                tracing::warn!("[yinyue] model '{m}' unavailable; using {}", engine.model_id);
            }
        }

        // First turn of a freshly rolled session: bridge the day/size roll with
        // a one-line "Previously" note so a thread mid-flight doesn't snap.
        // Deeper continuity rides shared memory (auto-recall + core), injected
        // by the turn core.
        seed_previously_if_fresh(state, &mut engine, &session_id);

        let ctx = crate::server::chat::ChatRunCtx {
            state: state.clone(),
            manager: state.manager.clone(),
            events_tx: state.events_tx.clone(),
            root: root.clone(),
            agent_id: YINYUE_AGENT.to_string(),
            session_id: Some(session_id.clone()),
            clean_msg: task,
            images: Vec::new(),
            policy: crate::engine::session_policy::SessionPolicy::owner(),
        };
        crate::server::chat::run_session_turn(
            &ctx,
            &mut engine,
            &state.manager,
            Some(YINYUE_MAX_LIVE_MSGS),
        )
        .await;

        engine.set_run_id(None);
        engine.last_assistant_text.clone()
    };

    // The turn core handles + persists its own errors; record the run as
    // completed and let an empty reply mean "nothing to say".
    let _ = state
        .manager
        .finish_agent_run(&run_id, crate::engine::agent::AgentRunStatus::Completed, None)
        .await;

    spoken.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Resolve Yinyue's model from the `pet.model` setting. An explicit id wins;
/// "auto" uses the metered Linggen Cloud model for signed-in (paid/free) users
/// and leaves the engine default for BYOK users (who pick their own). Returns
/// `None` to mean "keep the engine's current model".
fn resolve_pet_model(setting: &str) -> Option<String> {
    let s = setting.trim();
    if !s.is_empty() && !s.eq_ignore_ascii_case("auto") {
        return Some(s.to_string());
    }
    if crate::account::resolve_token().is_some() {
        Some(CLOUD_DEFAULT_MODEL.to_string())
    } else {
        None
    }
}

/// Pick Yinyue's current rolling session id: one per calendar day, rolling to an
/// extra segment when the active day's live engine nears its context limit. A
/// fresh day (or an unloaded session) always has headroom; continuity across
/// rolls rides shared memory, not the transcript.
async fn resolve_current_session(state: &Arc<ServerState>) -> String {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let base = format!("{YINYUE_SESSION_PREFIX}-{today}");
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
fn ensure_session_exists(state: &Arc<ServerState>, sid: &str, root: &std::path::Path) {
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
        .strip_prefix(&format!("{YINYUE_SESSION_PREFIX}-"))
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
fn seed_previously_if_fresh(
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
fn last_spoken_line(state: &Arc<ServerState>, current_sid: &str) -> Option<String> {
    let store = &state.manager.global_sessions;
    let mut sessions = store.list_sessions().ok()?;
    sessions.retain(|m| m.id.starts_with(YINYUE_SESSION_PREFIX) && m.id != current_sid);
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

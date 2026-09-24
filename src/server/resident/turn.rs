//! One turn of hers, through the shared turn-core, on her rolling session
//! and her model.

use super::*;

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
pub(crate) async fn run_yinyue_turn(
    state: &Arc<ServerState>,
    task: String,
    trigger_source: &str,
) -> Option<String> {
    let root = crate::util::resolve_path(std::path::Path::new("~/.linggen"));

    // Pet settings (Settings → General → Pet). Disabled → she doesn't run at all.
    let pet = state.manager.get_config_snapshot().await.pet;
    if !pet.enabled {
        return None;
    }

    // Rolling session: one per day, segmented when a day's thread fills up.
    let session_id = resolve_current_session(state).await;
    ensure_session_exists(state, &session_id, &root);

    // Engine-authored kickoffs (heralds, agent_chat, asked) carry the
    // spoken-line contract; a person's own words are never appended to.
    let task = with_contract(task, trigger_source);
    let seat = Seat {
        session_id,
        root,
        guest: false,
    };
    run_at(state, &seat, &pet, task, trigger_source).await
}

/// Run her turn as a guest in another session — an app's chat where the user
/// addressed her (`@银月 …`). The message is already in that session; she
/// reads its transcript, answers there in her own voice, with her own tools:
/// the session's skill is not hers to take up (`ChatRunCtx::guest`).
pub(crate) async fn run_guest_turn(
    state: &Arc<ServerState>,
    session_id: String,
    root: std::path::PathBuf,
    message: String,
) -> Option<String> {
    let pet = state.manager.get_config_snapshot().await.pet;
    if !pet.enabled {
        return None;
    }
    let seat = Seat {
        session_id,
        root,
        guest: true,
    };
    run_at(state, &seat, &pet, message, "user").await
}

/// Where one of her turns runs: her own rolling thread, or a guest seat at
/// another session's table.
struct Seat {
    session_id: String,
    root: std::path::PathBuf,
    guest: bool,
}

/// One turn of hers at `seat`, through the shared turn-core, on her model.
/// Returns her final text, trimmed; `None` when she produced none.
async fn run_at(
    state: &Arc<ServerState>,
    seat: &Seat,
    pet: &crate::config::PetConfig,
    task: String,
    trigger_source: &str,
) -> Option<String> {
    let Seat {
        session_id, root, ..
    } = seat;
    let agent = match state
        .manager
        .get_or_create_session_agent(session_id, root, YINYUE_AGENT)
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
            root,
            Some(session_id.as_str()),
            YINYUE_AGENT,
            None,
            Some(YINYUE_AGENT.to_string()),
        )
        .await
        .unwrap_or_else(|_| format!("run-{YINYUE_AGENT}-fallback"));

    let spoken = {
        let mut engine = agent.lock().await;
        // Her own thread: persist the incoming message to the session store so
        // it survives reload and the turn-core's restore sees a complete
        // thread. (The turn core only mirrors it into in-memory history.)
        // Inside the lock: two wakes racing for her engine keep each kickoff
        // next to its own turn. A guest's message is already on the table.
        if !seat.guest {
            crate::server::chat::helpers::persist_message_only(
                &state.manager,
                root,
                YINYUE_AGENT,
                "user",
                YINYUE_AGENT,
                &task,
                Some(session_id),
                false,
            )
            .await;
        }
        // Loop-break: if this turn was woken by an agent_chat, mark the session so
        // the agent_chat tool refuses to relay onward (one hop; user re-arms).
        // Mark/clear INSIDE the lock so the flag's lifetime matches exactly the
        // turn holding the engine — a concurrent same-session turn can't observe
        // or clear another turn's mark.
        if trigger_source == "agent_chat" {
            state.manager.mark_agent_chat_session(session_id);
        }
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
        // A guest leaves no recall rows on someone else's table.
        if seat.guest && engine.prompt_profile.include_memory {
            engine.prompt_profile.include_memory = false;
            engine.cached_system_prompt = None;
        }

        // Pick her brain per the Pet model setting (tier-aware default: the
        // metered Linggen Cloud model for signed-in users, the engine default
        // for BYOK). An unavailable id falls back to whatever she's already on.
        if let Some(m) = resolve_pet_model(&pet.model) {
            if engine.model_manager.has_model(&m) {
                engine.model_id = m;
            } else {
                tracing::warn!(
                    "[yinyue] model '{m}' unavailable; using {}",
                    engine.model_id
                );
            }
        }

        // First turn of a freshly rolled session: bridge the day/size roll with
        // a one-line "Previously" note so a thread mid-flight doesn't snap.
        // Deeper continuity rides shared memory (auto-recall + core), injected
        // by the turn core.
        if !seat.guest {
            seed_previously_if_fresh(state, &mut engine, session_id);
        }

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
            sender: None,
            guest: seat.guest,
            silence_ok: false,
        };
        crate::server::chat::run_session_turn(
            &ctx,
            &mut engine,
            &state.manager,
            Some(YINYUE_MAX_LIVE_MSGS),
        )
        .await;

        engine.set_run_id(None);
        if trigger_source == "agent_chat" {
            state.manager.clear_agent_chat_session(session_id);
        }
        engine.last_assistant_text.clone()
    };

    // The turn core handles + persists its own errors; record the run as
    // completed and let an empty reply mean "nothing to say".
    let _ = state
        .manager
        .finish_agent_run(
            &run_id,
            crate::engine::agent::AgentRunStatus::Completed,
            None,
        )
        .await;

    spoken
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Resolve Yinyue's model from the `pet.model` setting. An explicit id wins;
/// "auto" uses the metered Linggen Cloud model for signed-in (paid/free) users
/// and leaves the engine default for BYOK users (who pick their own). Returns
/// `None` to mean "keep the engine's current model".
pub(super) fn resolve_pet_model(setting: &str) -> Option<String> {
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

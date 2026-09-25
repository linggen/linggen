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
    run_home(state, task, None, trigger_source, Reach::Open).await
}

/// Her turn on her own thread, woken by an app moment: her line is all she
/// gives — spoken, and landed in the app's chat by the moment path — or
/// SILENT. She reaches no other agent from it ([`Reach::Sealed`]).
/// `aside` is what she reads for this turn only — the app chat's dialogue —
/// and never kept on her thread.
pub(crate) async fn run_moment_turn(
    state: &Arc<ServerState>,
    task: String,
    aside: Option<String>,
    trigger_source: &str,
) -> Option<String> {
    run_home(state, task, aside, trigger_source, Reach::Sealed).await
}

/// What a turn of hers may reach past her own line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Reach {
    /// Her ordinary turns: she may message another agent (`agent_chat`).
    Open,
    /// An app moment or a guest seat: she speaks, or is SILENT. The one
    /// exchange with an app's agent is the moment's `converse` path, never
    /// hers to start — a message from her lands in the app's chat and wakes
    /// that agent (2026-09-24: an idle moment's turn relayed its kickoff to
    /// Ling, which started her loop in the Lingjing chat).
    Sealed,
}

/// The tools a sealed turn withholds: the ones that reach another agent.
const SEALED_WITHHOLDS: &[&str] = &["agent_chat"];

/// The tools withheld from a turn with this reach.
pub(super) fn withheld_for(reach: Reach) -> std::collections::HashSet<String> {
    match reach {
        Reach::Open => Default::default(),
        Reach::Sealed => SEALED_WITHHOLDS.iter().map(|t| t.to_string()).collect(),
    }
}

/// Her turn on her own rolling thread, with the given reach.
async fn run_home(
    state: &Arc<ServerState>,
    task: String,
    aside: Option<String>,
    trigger_source: &str,
    reach: Reach,
) -> Option<String> {
    let root = her_root();

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
        reach,
    };
    run_at(state, &seat, &pet, (task, aside), trigger_source).await
}

/// Run her turn as a guest in another session — an app's chat where the user
/// addressed her (`@银月 …`). The message is already in that session; she
/// reads its transcript, answers there in her own voice, with her own tools:
/// the session's skill is not hers to take up (`ChatRunCtx::guest`).
pub(crate) async fn run_guest_turn(
    state: &Arc<ServerState>,
    session_id: String,
    message: String,
) -> Option<String> {
    let pet = state.manager.get_config_snapshot().await.pet;
    if !pet.enabled {
        return None;
    }
    // She works from her own folder at any table — the session's cwd and its
    // grants are its agent's, not hers (`seat_permissions`).
    let seat = Seat {
        session_id,
        root: her_root(),
        guest: true,
        reach: Reach::Sealed,
    };
    run_at(state, &seat, &pet, (message, None), "user").await
}

/// Her own folder: where her turns run, and what her permissions cover.
fn her_root() -> std::path::PathBuf {
    crate::util::resolve_path(std::path::Path::new("~/.linggen"))
}

/// Where one of her turns runs: her own rolling thread, or a guest seat at
/// another session's table.
struct Seat {
    session_id: String,
    root: std::path::PathBuf,
    guest: bool,
    reach: Reach,
}

/// Her engine for a seat. Her own thread is her session's engine. A guest
/// seat gets a fresh engine of hers: a session holds ONE engine, built for
/// the agent that runs it — asking it for hers in an app's chat hands back
/// Ling's, with her prompt and the app's tools (seen 2026-09-24). A fresh one
/// costs nothing a guest keeps: her thread there is rebuilt each turn.
async fn engine_for(
    state: &Arc<ServerState>,
    seat: &Seat,
) -> anyhow::Result<Arc<tokio::sync::Mutex<crate::engine::AgentEngine>>> {
    if !seat.guest {
        return state
            .manager
            .get_or_create_session_agent(&seat.session_id, &seat.root, YINYUE_AGENT)
            .await;
    }
    let mut engine = state
        .manager
        .spawn_delegation_engine(&seat.root, YINYUE_AGENT)
        .await?;
    // Top level, not a delegate: her reply is persisted to the table.
    let max_depth = state
        .manager
        .get_config_snapshot()
        .await
        .agent
        .max_delegation_depth;
    engine.set_delegation_depth(0, max_depth);
    engine.seat_permissions = Some(guest_permissions(&engine));
    Ok(Arc::new(tokio::sync::Mutex::new(engine)))
}

/// What she may do as a guest: exactly what her own sessions start with — the
/// configured default mode on her own folder — and never a prompt, since the
/// table's surface is not hers to ask on.
fn guest_permissions(
    engine: &crate::engine::AgentEngine,
) -> crate::engine::permission::SessionPermissions {
    crate::engine::permission::SessionPermissions::seat(
        &engine.tools.builtins.cwd().to_string_lossy(),
        engine.cfg.permission_mode,
    )
}

/// One turn of hers at `seat`, through the shared turn-core, on her model:
/// the task, and an aside read for this turn only. Returns her final text,
/// trimmed; `None` when she produced none.
async fn run_at(
    state: &Arc<ServerState>,
    seat: &Seat,
    pet: &crate::config::PetConfig,
    (task, aside): (String, Option<String>),
    trigger_source: &str,
) -> Option<String> {
    let Seat {
        session_id, root, ..
    } = seat;
    let agent = match engine_for(state, seat).await {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("[yinyue] could not create Yinyue agent: {e}");
            return None;
        }
    };

    // No run is begun here: the turn core tracks its own
    // (`run_loop_with_tracking`), and a second one around it was a second
    // "running" row per message — the stop button could pick the outer one,
    // which the engine never checks (seen 2026-09-24: yinyue01 + yinyue02).
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
        // Set every turn: her rolling engine outlives this one.
        engine.withheld_tools = withheld_for(seat.reach);
        // Clear so we read THIS turn's final line — the engine is reused across
        // turns and would otherwise hold the prior one.
        engine.last_assistant_text = None;

        // Her turn is a person's session, not a task: the owner policy's
        // profile, applied to the engine (a fresh engine's default profile
        // frames the turn as an autonomous task — a second "Task:" message).
        let policy = crate::engine::session_policy::SessionPolicy::owner();
        policy.apply(&mut engine);

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
            policy,
            sender: None,
            guest: seat.guest,
            silence_ok: false,
            aside,
        };
        crate::server::chat::run_session_turn(
            &ctx,
            &mut engine,
            &state.manager,
            Some(YINYUE_MAX_LIVE_MSGS),
        )
        .await;

        if trigger_source == "agent_chat" {
            state.manager.clear_agent_chat_session(session_id);
        }
        engine.last_assistant_text.clone()
    };

    // The turn core handles + persists its own errors and its run; an empty
    // reply means "nothing to say".
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A moment or guest turn cannot reach another agent; her own turns can.
    #[test]
    fn a_sealed_turn_withholds_agent_chat_and_an_open_one_nothing() {
        assert!(withheld_for(Reach::Sealed).contains("agent_chat"));
        assert!(withheld_for(Reach::Open).is_empty());
    }

    fn her_engine() -> crate::engine::AgentEngine {
        let cfg = crate::engine::EngineConfig::from_app_config(
            &crate::config::Config::default(),
            std::env::temp_dir(),
            crate::engine::InterfaceMode::Web,
        );
        let models =
            std::sync::Arc::new(crate::provider::models::ModelManager::new_with_credentials(
                Vec::new(),
                &crate::credentials::Credentials::default(),
            ));
        let mut engine = crate::engine::AgentEngine::new(
            cfg,
            models,
            "m".to_string(),
            crate::engine::AgentRole::Lead,
        )
        .unwrap();
        let (spec, body) = crate::extensions::agents::parse_agent_markdown(include_str!(
            "../../../agents/yinyue.md"
        ))
        .unwrap();
        engine.set_spec(YINYUE_AGENT.to_string(), spec, body);
        engine
    }

    /// Her turn is a person's: the request ends on the message itself, never
    /// wrapped a second time as an autonomous task (2026-09-25: every turn
    /// of hers carried "Autonomous agent loop started… Task:" — her engine
    /// kept the default profile because the owner policy was never applied).
    #[test]
    fn her_turn_ends_on_the_message_with_no_task_wrapper() {
        let mut engine = her_engine();
        assert!(
            engine.prompt_profile.task_bootstrap,
            "a fresh engine frames a task"
        );
        crate::engine::session_policy::SessionPolicy::owner().apply(&mut engine);
        engine
            .chat_history
            .push(crate::message::ChatMessage::new("user", "[User]: 你好"));
        let (messages, _, _) = engine.prepare_loop_messages(
            "[User]: 你好",
            true,
            crate::engine::prompt::PromptPurpose::Preview,
        );
        let last = messages.last().unwrap();
        assert_eq!(
            (last.role.as_str(), last.content.as_str()),
            ("user", "[User]: 你好")
        );
        assert!(!messages
            .iter()
            .any(|m| m.content.contains("Autonomous agent loop")));

        // And her turn code applies it, before anything narrows the profile.
        let src = include_str!("turn.rs");
        let apply = src.find(concat!("policy.apply", "(&mut engine)")).unwrap();
        let narrowed = src.find(concat!("engine.prompt_profile", ".")).unwrap();
        assert!(apply < narrowed, "the policy is applied first");
    }
}

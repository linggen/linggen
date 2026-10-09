//! One turn of hers, through the shared turn-core, on her model: on her own
//! rolling thread, or — woken by an app moment — at the app chat's table as
//! one of its members.

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
    run_home(state, task, trigger_source, Reach::Open).await
}

/// Her turn woken by an app moment: her line is all she gives — spoken — or
/// SILENT. She reaches no other agent from it ([`Reach::Sealed`]). `table`:
/// the app chat the moment names — her turn runs there, a member at its
/// table, reading its whole thread, and her line is a row of it (a moment is
/// a turn like any other, `doc/shared-session-spec.md`). `None`: on her own
/// thread.
pub(crate) async fn run_moment_turn(
    state: &Arc<ServerState>,
    (task, table): (String, Option<String>),
    trigger_source: &str,
) -> Option<String> {
    match table {
        Some(session_id) => run_at_table(state, session_id, task, trigger_source).await,
        None => run_home(state, task, trigger_source, Reach::Sealed).await,
    }
}

/// What a turn of hers may reach past her own line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Reach {
    /// Her ordinary turns: she may message another agent (`agent_chat`).
    Open,
    /// An app moment: she speaks, or is SILENT. The one exchange with an
    /// app's agent is the moment's `converse` path, never hers to start — a
    /// message from her lands in the app's chat and wakes that agent
    /// (2026-09-24: an idle moment's turn relayed its kickoff to Ling, which
    /// started her loop in the Lingjing chat).
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
    trigger_source: &str,
    reach: Reach,
) -> Option<String> {
    let root = her_root();

    // Pet settings (Settings → General → Pet). Disabled → she doesn't run at all.
    let pet = state.manager.get_config_snapshot().await.pet;
    if !pet.enabled || !here(state).await {
        return None;
    }

    // Rolling session: one per day, segmented when a day's thread fills up.
    let session_id = resolve_current_session(state).await;
    ensure_session_exists(state, &session_id, &root);

    // Engine-authored kickoffs (heralds, agent_chat, asked) carry the
    // spoken-line contract; a person's own words are never appended to.
    let task = with_contract(task, trigger_source);
    let turn = Turn {
        session_id,
        root,
        reach,
        home: true,
    };
    run_at(state, &turn, &pet, task, trigger_source).await
}

/// Her moment turn at an app chat's table: seated there (for good, as an
/// addressed member is), woken by a hidden kickoff that is hers alone —
/// another member never reads it (`chat::thread`).
async fn run_at_table(
    state: &Arc<ServerState>,
    session_id: String,
    task: String,
    trigger_source: &str,
) -> Option<String> {
    let pet = state.manager.get_config_snapshot().await.pet;
    if !pet.enabled {
        return None;
    }
    let meta = state
        .manager
        .global_sessions
        .get_session_meta(&session_id)
        .ok()??;
    // Not there (any more) in that app's world: no turn at its table — the
    // gate is read again as the wake comes, not only when the moment was
    // posted.
    if crate::server::chat::presence::absent_in_session(&state.manager, &session_id, YINYUE_AGENT)
        .await
    {
        return None;
    }
    crate::server::chat::members::seat(&state.manager, &session_id, YINYUE_AGENT).await;
    let root = meta
        .cwd
        .or(meta.project)
        .map(|c| crate::util::resolve_path(std::path::Path::new(&c)))
        .unwrap_or_else(her_root);
    let task = format!("[HIDDEN] {}", with_contract(task, trigger_source));
    let turn = Turn {
        session_id,
        root,
        reach: Reach::Sealed,
        home: false,
    };
    run_at(state, &turn, &pet, task, trigger_source).await
}

/// Her own folder: where her turns run, and what her permissions cover.
fn her_root() -> std::path::PathBuf {
    crate::util::resolve_path(std::path::Path::new("~/.linggen"))
}

/// Where one of her turns runs.
struct Turn {
    session_id: String,
    root: std::path::PathBuf,
    reach: Reach,
    /// Her own rolling thread (kept short, seeded across a roll) — else an
    /// app chat's table.
    home: bool,
}

/// One turn of hers, through the shared turn-core, on her model. Returns
/// her final text, trimmed; `None` when she produced none.
async fn run_at(
    state: &Arc<ServerState>,
    turn: &Turn,
    pet: &crate::config::PetConfig,
    task: String,
    trigger_source: &str,
) -> Option<String> {
    let Turn {
        session_id, root, ..
    } = turn;
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

    // No run is begun here: the turn core tracks its own
    // (`run_loop_with_tracking`), and a second one around it was a second
    // "running" row per message — the stop button could pick the outer one,
    // which the engine never checks (seen 2026-09-24: yinyue01 + yinyue02).
    let spoken = {
        let _held = state.manager.session_turn(session_id).lock_owned().await;
        let mut engine = agent.lock().await;
        // Persist the incoming message to the session store so it survives
        // reload and every member's thread holds it. (The turn core only
        // mirrors it into in-memory history.) Inside the lock: two wakes
        // racing keep each kickoff next to its own turn.
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
        // Loop-break: if this turn was woken by an agent_chat, mark the session so
        // the agent_chat tool refuses to relay onward (one hop; user re-arms).
        // Mark/clear INSIDE the lock so the flag's lifetime matches exactly the
        // turn holding the engine — a concurrent same-session turn can't observe
        // or clear another turn's mark.
        if trigger_source == "agent_chat" {
            state.manager.mark_agent_chat_session(session_id);
        }
        let policy = ready_for_turn(&mut engine, turn, pet);
        apply_her_model(state, &mut engine, session_id, pet).await;
        // At an app's table the session's recall policy is the app's: no
        // biography, no core block (`handler::turn_creator`).
        if !turn.home && table_is_not_a_persons(state, session_id) {
            engine.prompt_profile.include_memory = false;
        }

        // First turn of a freshly rolled session: bridge the day/size roll with
        // a one-line "Previously" note so a thread mid-flight doesn't snap.
        // Deeper continuity rides shared memory (auto-recall + core), injected
        // by the turn core.
        if turn.home {
            seed_previously_if_fresh(state, &mut engine, session_id);
        }

        let ctx = crate::server::chat::ChatRunCtx {
            via: None,
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
            // At a table, silence is an answer and leaves no row there.
            silence_ok: !turn.home && trigger_source != "asked",
        };
        let cap = turn.home.then_some(YINYUE_MAX_LIVE_MSGS);
        crate::server::chat::run_session_turn(&ctx, &mut engine, &state.manager, cap).await;

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

/// Whether the session at this table belongs to a skill or a mission — not a
/// person's own chat.
fn table_is_not_a_persons(state: &Arc<ServerState>, session_id: &str) -> bool {
    state
        .manager
        .global_sessions
        .get_session_meta(session_id)
        .ok()
        .flatten()
        .is_some_and(|m| m.skill.is_some() || m.mission_id.is_some())
}

/// Her model in this session: the one the session keeps for her, else her
/// Pet setting (`pet.model`), else whatever she is on — each only when this
/// machine has it.
async fn apply_her_model(
    state: &Arc<ServerState>,
    engine: &mut crate::engine::AgentEngine,
    session_id: &str,
    pet: &crate::config::PetConfig,
) {
    let members = crate::server::chat::members::of_session(&state.manager, session_id).await;
    let kept = members
        .iter()
        .find(|m| m.id == YINYUE_AGENT)
        .and_then(|m| m.model.clone());
    let wanted = kept.or_else(|| resolve_pet_model(&pet.model));
    let Some(m) = wanted else {
        return;
    };
    match engine.model_manager.resolve_id(&m) {
        Some(live) => engine.model_id = live,
        None => tracing::warn!(
            "[yinyue] model '{m}' unavailable; using {}",
            engine.model_id
        ),
    }
}

/// Set her engine up for one turn. Idempotent — her engine outlives the
/// turn, and this picks up live settings edits. Returns the policy the turn
/// runs under.
fn ready_for_turn(
    engine: &mut crate::engine::AgentEngine,
    turn: &Turn,
    pet: &crate::config::PetConfig,
) -> crate::engine::session_policy::SessionPolicy {
    engine.set_parent_agent(None);
    engine.withheld_tools = withheld_for(turn.reach);
    // Clear so we read THIS turn's final line — the engine is reused across
    // turns and would otherwise hold the prior one.
    engine.last_assistant_text = None;

    // Her turn is a person's session, not a task: the owner policy's
    // profile, applied to the engine (a fresh engine's default profile
    // frames the turn as an autonomous task — a second "Task:" message).
    let policy = crate::engine::session_policy::SessionPolicy::owner();
    policy.apply(engine);

    // Her memory tools act for the session she speaks in: a write is stamped
    // with it (`source_session`), and a table whose skill keeps memory holds
    // her reads and writes to that skill's dir, as it holds its own agent's
    // (`memory_mcp::memory_place`).
    engine
        .tools
        .builtins
        .set_session_id(Some(turn.session_id.clone()));
    tune_companion(engine, pet);
    policy
}

/// Her recall, tuned from the Pet settings (default: one high-relevance
/// record at ≥0.8) — on her engine's own cfg, in any session she speaks in,
/// so Ling's full-store recall is untouched.
pub(crate) fn tune_companion(
    engine: &mut crate::engine::AgentEngine,
    pet: &crate::config::PetConfig,
) {
    engine.cfg.memory_recall_count = pet.recall_count.max(1);
    engine.cfg.memory_inject_min_score = Some(pet.recall_min_score);
}

/// Resolve Yinyue's model from the `pet.model` setting. An explicit id wins;
/// "auto" uses the metered Linggen Cloud model for signed-in (paid/free) users
/// and leaves the engine default for BYOK users (who pick their own). Returns
/// `None` to mean "keep the engine's current model".
pub(crate) fn resolve_pet_model(setting: &str) -> Option<String> {
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

    /// A moment turn cannot reach another agent; her own turns can.
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

    fn turn(home: bool) -> Turn {
        Turn {
            session_id: "sess-1758700000-cfo".to_string(),
            root: std::env::temp_dir(),
            reach: Reach::Sealed,
            home,
        }
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
        ready_for_turn(&mut engine, &turn(false), &Default::default());
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
    }

    /// At a table her memory tools act for that session — its skill's
    /// memory context holds them and her writes are stamped with it — and
    /// a moment there reaches no other agent.
    #[test]
    fn at_a_table_her_tools_act_for_that_session() {
        let mut engine = her_engine();
        ready_for_turn(&mut engine, &turn(false), &Default::default());
        assert_eq!(
            engine.tools.builtins.session_id.as_deref(),
            Some("sess-1758700000-cfo")
        );
        assert!(engine.withheld_tools.contains("agent_chat"));
    }

    /// Her recall is persisted as a row wherever she speaks: every member
    /// reads it as a note (no guest exemption any more).
    #[test]
    fn her_recall_is_a_row_of_the_session_wherever_she_speaks() {
        let runtime = include_str!("../chat/runtime.rs");
        assert!(!runtime.contains(concat!("ctx", ".guest")));
        assert!(runtime.contains(concat!("\"memory", "-recall\",")));
    }
}

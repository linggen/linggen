use crate::missions::{self, Mission, MissionRunEntry, MISSION_AGENT_ID};
use crate::server::{ServerEvent, ServerState};
use chrono::Local;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use tracing::{debug, info, warn};

/// How often the scheduler checks missions (seconds).
const CHECK_INTERVAL_SECS: u64 = 10;

/// Maximum triggers per mission per day to prevent runaway cost.
const MAX_TRIGGERS_PER_DAY: u32 = 100;

/// The built-in memory-consolidation mission. Its id is its directory
/// name under `~/.linggen/missions/`. Firing it does NOT run a generic
/// `ling` agent prompt — the worklist/TTL is engine-owned, so a static
/// mission body can't carry it. Instead it invokes the engine's global
/// consolidate+evict pass directly (see [`dispatch_dream_mission`]).
const DREAM_MISSION_ID: &str = "dream";

/// Per-mission tracking state.
struct MissionState {
    /// Last minute we fired this mission (to dedup within the same minute).
    last_fire_minute: Option<i64>,
    /// Daily trigger count + the date it applies to.
    daily_count: u32,
    daily_date: Option<chrono::NaiveDate>,
    /// True while a dispatched mission run is still executing.
    running: Arc<std::sync::atomic::AtomicBool>,
}

impl MissionState {
    fn new() -> Self {
        Self {
            last_fire_minute: None,
            daily_count: 0,
            daily_date: None,
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Reset daily count if the date has changed.
    fn maybe_reset_daily(&mut self, today: chrono::NaiveDate) {
        if self.daily_date != Some(today) {
            self.daily_count = 0;
            self.daily_date = Some(today);
        }
    }
}

/// Background loop that evaluates cron missions and triggers agent runs.
pub async fn mission_scheduler_loop(state: Arc<ServerState>) {
    let mut interval = time::interval(Duration::from_secs(CHECK_INTERVAL_SECS));
    let mut mission_states: HashMap<String, MissionState> = HashMap::new();

    loop {
        interval.tick().await;

        let now = Local::now();
        let today = now.date_naive();
        let current_minute = now.timestamp() / 60;

        let enabled_missions = match state.manager.missions.list_enabled_missions() {
            Ok(m) => m,
            Err(e) => {
                debug!("Mission scheduler: failed to list missions: {}", e);
                continue;
            }
        };

        for mission in &enabled_missions {
            let mission_key = &mission.id;

            let ms = mission_states
                .entry(mission_key.clone())
                .or_insert_with(MissionState::new);
            ms.maybe_reset_daily(today);

            // Check daily trigger cap
            if ms.daily_count >= MAX_TRIGGERS_PER_DAY {
                debug!(
                    "Mission scheduler: mission '{}' hit daily cap ({}), skipping",
                    mission.id, MAX_TRIGGERS_PER_DAY
                );
                continue;
            }

            // Check dedup: don't fire twice in the same minute
            if ms.last_fire_minute == Some(current_minute) {
                continue;
            }

            // Check if cron matches current time
            if !cron_matches_now(&mission.schedule, &now) {
                continue;
            }

            // Missions are a first-class subsystem — no "mission" skill is
            // required. Per-mission dependencies are declared in `requires:`
            // and checked at dispatch by find_missing_requires.

            // Working dir: `cwd` → legacy `project` → env cwd, with
            // `~`/`$VAR` expansion (so the agent's Bash tool can spawn in it).
            let (root, project_path) = mission_root(mission);

            // Busy-skip: if previous run is still executing, skip and log.
            if ms.running.load(std::sync::atomic::Ordering::Relaxed) {
                info!(
                    "Mission scheduler: mission '{}' still running, skipping trigger",
                    mission.id
                );
                let skip_id = format!(
                    "mission-run-{}-{}",
                    crate::util::now_ts_secs(),
                    &uuid::Uuid::new_v4().to_string()[..8]
                );
                record_mission_run(&state, mission, &skip_id, None, "skipped", true);
                ms.last_fire_minute = Some(current_minute);
                continue;
            }

            // Fire!
            ms.last_fire_minute = Some(current_minute);
            ms.daily_count += 1;

            info!(
                "Mission scheduler: triggering mission '{}' (cwd: {:?})",
                mission.id, mission.cwd
            );

            // Agent dispatch. Entry-script pre-stage lands in Phase 2 — today
            // every mission runs the agent loop directly.
            ms.running.store(true, std::sync::atomic::Ordering::Relaxed);

            state
                .manager
                .update_agent_activity(&project_path, MISSION_AGENT_ID)
                .await;

            let state_clone = state.clone();
            let mission_owned = mission.clone();
            let project_path_owned = project_path.clone();
            let root_owned = root.clone();
            let running_flag = ms.running.clone();

            tokio::spawn(async move {
                dispatch_mission_prompt(
                    state_clone,
                    root_owned,
                    &project_path_owned,
                    &mission_owned,
                    None,
                )
                .await;
                running_flag.store(false, std::sync::atomic::Ordering::Relaxed);
            });
        }

        // Clean up state for missions that no longer exist
        let active_keys: std::collections::HashSet<String> = enabled_missions
            .iter()
            .map(|m| m.id.clone())
            .collect();
        mission_states.retain(|k, _| active_keys.contains(k));
    }
}

/// Check if a cron expression matches the current time (within the current minute).
fn cron_matches_now(schedule: &str, now: &chrono::DateTime<Local>) -> bool {
    let cron_schedule = match missions::parse_cron(schedule) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let one_min_ago = *now - chrono::Duration::seconds(60);
    if let Some(next) = cron_schedule.after(&one_min_ago).next() {
        let next_minute = next.timestamp() / 60;
        let now_minute = now.timestamp() / 60;
        next_minute == now_minute
    } else {
        false
    }
}

/// Mission session title — `{name} session`, mirroring skill sessions.
/// Falls back to the mission id when no friendly name is set. The mission
/// badge in the UI already labels these as missions, and the list shows
/// relative time, so the title stays short.
fn mission_session_title(mission: &Mission) -> String {
    let label = mission.name.as_deref().unwrap_or(&mission.id);
    format!("{} session", label)
}

/// Create a new session for a mission run in the global session store.
pub fn create_mission_session(mission: &Mission) -> Option<String> {
    let session_id = format!(
        "sess-{}-{}",
        crate::util::now_ts_secs(),
        &uuid::Uuid::new_v4().to_string()[..8]
    );
    let store = crate::state_fs::SessionStore::with_sessions_dir(
        crate::paths::global_sessions_dir(),
    );
    let mission_cwd = mission.cwd.clone().or_else(|| mission.project.clone());
    let meta = crate::state_fs::sessions::SessionMeta {
        id: session_id.clone(),
        title: mission_session_title(mission),
        created_at: crate::util::now_ts_secs(),
        // Missions are a first-class subsystem; they don't bind a skill.
        // `creator: "mission"` alone distinguishes mission sessions.
        skill: None,
        creator: "mission".into(),
        cwd: mission_cwd.clone(),
        project: mission_cwd.clone(),
        project_name: mission_cwd.as_ref().and_then(|p| {
            std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().to_string())
        }),
        mission_id: Some(mission.id.clone()),
        // Pin the mission's configured model onto the session so the UI header
        // shows the right model and follow-up chat turns (which go through
        // chat_api with the session's model_id) don't reset back to the global
        // default.
        model_id: mission.model.clone(),
        user_id: None,
        compact_threshold: None,
        compact_focus: None,
    };
    match store.add_session(&meta) {
        Ok(_) => Some(session_id),
        Err(e) => {
            warn!("Mission scheduler: failed to create session: {}", e);
            None
        }
    }
}

/// Public wrapper for triggering a mission manually (from API).
/// Accepts an optional pre-created `session_id` so the caller can return it immediately.
pub async fn dispatch_mission_prompt_public(
    state: Arc<ServerState>,
    root: std::path::PathBuf,
    project_path: &str,
    mission: &Mission,
    session_id: Option<String>,
) {
    dispatch_mission_prompt(state, root, project_path, mission, session_id).await;
}

/// Run the built-in `dream` mission: the engine's global consolidate +
/// evict pass. Not a generic agent prompt — the TTL/worklist is
/// engine-owned (locked contract), which a static mission body can't
/// carry. We still create the mission session, emit the same events, and
/// write a `MissionRunEntry` so it shows up in mission history exactly
/// like any mission (the audit trail the user asked for); the heavy
/// lifting is `run_consolidate_evict`, which spawns its own `ling-mem`
/// CONSOLIDATE subagent with its own run record.
async fn dispatch_dream_mission(
    state: Arc<ServerState>,
    root: std::path::PathBuf,
    project_path: &str,
    mission: &Mission,
    pre_session_id: Option<String>,
) {
    use crate::server::AgentStatusKind;
    use std::sync::atomic::Ordering;

    let agent_id = MISSION_AGENT_ID;
    let mission_run_id = format!(
        "mission-run-{}-{}",
        crate::util::now_ts_secs(),
        &uuid::Uuid::new_v4().to_string()[..8]
    );

    // Shared overlap guard: both the cron fire and the turn-seam catch-up
    // funnel through here. `swap(true)` — if it was already true, another
    // dream run owns it; record a skip and bail WITHOUT clearing (the
    // owner clears). Only the acquirer clears, at the end.
    if state.dream_running.swap(true, Ordering::SeqCst) {
        info!("dream mission: already running, skipping this trigger");
        record_mission_run(&state, mission, &mission_run_id, None, "skipped", true);
        return;
    }

    let has_pre_session = pre_session_id.is_some();
    let session_id = pre_session_id.or_else(|| create_mission_session(mission));

    if !has_pre_session {
        if let Some(ref sid) = session_id {
            let evt_cwd = mission
                .cwd
                .clone()
                .or_else(|| mission.project.clone())
                .unwrap_or_default();
            let _ = state.events_tx.send(ServerEvent::SessionCreated {
                session_id: sid.clone(),
                title: mission_session_title(mission),
                creator: "mission".into(),
                project: Some(evt_cwd.clone()),
                project_name: std::path::Path::new(&evt_cwd)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string()),
                skill: None,
                mission_id: Some(mission.id.clone()),
            });
            // Persist a kickoff line so the session chat documents what ran.
            let global_store = crate::state_fs::SessionStore::with_sessions_dir(
                crate::paths::global_sessions_dir(),
            );
            let _ = global_store.add_chat_message(
                sid,
                &crate::state_fs::sessions::ChatMsg {
                    agent_id: agent_id.to_string(),
                    from_id: "user".to_string(),
                    to_id: agent_id.to_string(),
                    content: "Memory dream: consolidate past-TTL episodic memories and evict the rest.".to_string(),
                    timestamp: crate::util::now_ts_secs(),
                    is_observation: false,
                },
            );
        }
    }

    let _ = state.events_tx.send(ServerEvent::MissionTriggered {
        mission_id: mission.id.clone(),
        agent_id: agent_id.to_string(),
        project_root: project_path.to_string(),
        session_id: session_id.clone(),
    });
    state
        .send_agent_status(
            agent_id.to_string(),
            AgentStatusKind::Working,
            Some("Memory consolidation".to_string()),
            None,
            session_id.clone(),
        )
        .await;

    // TTL policy is engine-owned. Read the live config; fall back to the
    // built-in default if it can't be loaded.
    let episodic_ttl_days = crate::config::Config::load_with_path()
        .map(|(c, _)| c.agent.episodic_ttl_days)
        .unwrap_or_else(|_| crate::config::Config::default().agent.episodic_ttl_days);

    // The consolidate+evict subagent is keyed to this mission session so
    // its run record / events are auditable under the mission.
    let sid_for_call = session_id
        .clone()
        .unwrap_or_else(|| mission_run_id.clone());
    let result = crate::server::run_consolidate_evict(
        &state,
        &state.manager,
        &sid_for_call,
        agent_id,
        &root,
        episodic_ttl_days,
    )
    .await;

    let status = match &result {
        Ok((promoted, superseded, deleted)) => {
            info!(
                "dream mission: consolidate ok (promoted={promoted} superseded={superseded} deleted={deleted})"
            );
            "completed"
        }
        Err(e) => {
            warn!("dream mission: consolidate failed: {e}");
            "failed"
        }
    };

    record_mission_run_full(
        &state,
        mission,
        &mission_run_id,
        session_id.as_deref(),
        status,
        false,
        None,
        None,
    );
    state
        .send_agent_status(
            agent_id.to_string(),
            AgentStatusKind::Idle,
            None,
            None,
            session_id.clone(),
        )
        .await;

    // Release the shared guard — only the acquirer reaches here.
    state.dream_running.store(false, Ordering::SeqCst);
}

/// Resolve a mission's working dir the same way the scheduler loop does:
/// `cwd` → legacy `project` → current dir, with `~`/`$VAR` expansion.
fn mission_root(mission: &Mission) -> (std::path::PathBuf, String) {
    let raw_cwd = mission
        .cwd
        .clone()
        .or_else(|| mission.project.clone())
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        });
    let root = crate::util::resolve_path(std::path::Path::new(&raw_cwd));
    let project_path = root.to_string_lossy().to_string();
    (root, project_path)
}

/// Turn-seam catch-up for the built-in `dream` mission. Called from the
/// post-turn seam (owner sessions only). The mission has a daily cron,
/// but cron is missed when the machine is off/asleep; this re-triggers it
/// the next time Linggen is used if it hasn't run within
/// `dream_catchup_hours`. Cheap + non-blocking: spawns, reads run
/// history, compares, and only then dispatches. Overlap with the cron
/// fire is prevented by the shared `dream_running` guard in
/// [`dispatch_dream_mission`].
pub(crate) fn maybe_fire_dream_catchup(state: Arc<ServerState>) {
    tokio::spawn(async move {
        // Respect user intent: a missing or disabled `dream` mission means
        // the user opted out of auto-consolidation — no catch-up.
        let mission = match state.manager.missions.get_mission(DREAM_MISSION_ID) {
            Ok(Some(m)) if m.enabled => m,
            Ok(_) => return,
            Err(e) => {
                debug!("dream catch-up: get_mission failed: {e}");
                return;
            }
        };

        // Last *attempt* (completed or failed; skipped doesn't count) by
        // trigger time. None ⇒ never run ⇒ overdue.
        let last_run_secs = match state.manager.missions.list_mission_runs(DREAM_MISSION_ID) {
            Ok(runs) => runs
                .iter()
                .filter(|r| !r.skipped)
                .map(|r| r.triggered_at)
                .max(),
            Err(e) => {
                debug!("dream catch-up: list_mission_runs failed: {e}");
                return;
            }
        };

        let catchup_hours = crate::config::Config::load_with_path()
            .map(|(c, _)| c.agent.dream_catchup_hours)
            .unwrap_or_else(|_| crate::config::Config::default().agent.dream_catchup_hours);
        let threshold_secs = catchup_hours.saturating_mul(3600);
        let now = crate::util::now_ts_secs();

        let overdue = match last_run_secs {
            None => true,
            Some(last) => now.saturating_sub(last) >= threshold_secs,
        };
        if !overdue {
            return;
        }

        info!(
            "dream catch-up: last run {:?}s ago (threshold {}h) — triggering",
            last_run_secs.map(|l| now.saturating_sub(l)),
            catchup_hours
        );
        let (root, project_path) = mission_root(&mission);
        dispatch_mission_prompt_public(state.clone(), root, &project_path, &mission, None).await;
    });
}

/// Dispatch a mission prompt to the mission agent.
async fn dispatch_mission_prompt(
    state: Arc<ServerState>,
    root: std::path::PathBuf,
    project_path: &str,
    mission: &Mission,
    pre_session_id: Option<String>,
) {
    use crate::server::AgentStatusKind;

    // The built-in memory mission is engine-driven, not a generic agent
    // prompt — branch before any agent/prompt setup.
    if mission.id == DREAM_MISSION_ID {
        dispatch_dream_mission(state, root, project_path, mission, pre_session_id).await;
        return;
    }

    let agent_id = MISSION_AGENT_ID;

    // Mission-level run id. Used for the per-run output dir, the
    // MISSION_RUN_ID env var, and the MissionRunEntry.run_id.
    let mission_run_id = format!(
        "mission-run-{}-{}",
        crate::util::now_ts_secs(),
        &uuid::Uuid::new_v4().to_string()[..8]
    );

    // Fast-fail: requires-check. A missing capability → no point running.
    if let Some(missing) = find_missing_requires(&state, mission).await {
        warn!(
            "Mission scheduler: mission '{}' requires '{}' which is not registered — skipping",
            mission.id, missing
        );
        record_mission_run_full(
            &state,
            mission,
            &mission_run_id,
            None,
            "failed",
            false,
            None,
            None,
        );
        return;
    }

    // Create the per-run output directory. Entry script writes here; agent
    // reads files from here. See doc/mission-spec.md → Entry script contract.
    let output_dir = state
        .manager
        .missions
        .mission_dir(&mission.id)
        .join("runs")
        .join(&mission_run_id);
    if let Err(e) = std::fs::create_dir_all(&output_dir) {
        warn!(
            "Mission scheduler: failed to create output dir for '{}': {}",
            mission.id, e
        );
        record_mission_run_full(
            &state,
            mission,
            &mission_run_id,
            None,
            "failed",
            false,
            None,
            Some(output_dir.to_string_lossy().into_owned()),
        );
        return;
    }

    // Entry script pre-stage. Runs before the agent (if any). Non-zero exit
    // aborts the mission run before the agent is created.
    let entry_exit_code = if let Some(ref entry) = mission.entry {
        match run_entry_script(mission, entry, &output_dir, &mission_run_id, &state).await {
            Ok(code) => Some(code),
            Err(e) => {
                warn!("Mission scheduler: entry script error for '{}': {}", mission.id, e);
                record_mission_run_full(
                    &state,
                    mission,
                    &mission_run_id,
                    None,
                    "failed",
                    false,
                    Some(-1),
                    Some(output_dir.to_string_lossy().into_owned()),
                );
                return;
            }
        }
    } else {
        None
    };

    if matches!(entry_exit_code, Some(c) if c != 0) {
        record_mission_run_full(
            &state,
            mission,
            &mission_run_id,
            None,
            "failed",
            false,
            entry_exit_code,
            Some(output_dir.to_string_lossy().into_owned()),
        );
        return;
    }

    // Script-only mission: entry ran successfully and there's no agent prompt.
    // No session, no agent loop — just record completion.
    if mission.prompt.trim().is_empty() {
        record_mission_run_full(
            &state,
            mission,
            &mission_run_id,
            None,
            "completed",
            false,
            entry_exit_code,
            Some(output_dir.to_string_lossy().into_owned()),
        );
        return;
    }

    // Use pre-created session or create a new one
    let has_pre_session = pre_session_id.is_some();
    let session_id = pre_session_id.or_else(|| create_mission_session(mission));

    let sid = session_id.as_deref().unwrap_or("default");
    let agent = match state.manager.get_or_create_session_agent(sid, &root, agent_id).await {
        Ok(a) => a,
        Err(e) => {
            warn!(
                "Mission scheduler: failed to get mission agent: {}",
                e
            );
            record_mission_run_full(
                &state,
                mission,
                &mission_run_id,
                None,
                "failed",
                false,
                entry_exit_code,
                Some(output_dir.to_string_lossy().into_owned()),
            );
            return;
        }
    };

    let mut engine = agent.lock().await;

    let manager = state.manager.clone();
    let events_tx = state.events_tx.clone();

    // Emit session_created so the unified session list updates in real-time
    if !has_pre_session {
        if let Some(ref sid) = session_id {
            let evt_cwd = mission.cwd.clone().or_else(|| mission.project.clone()).unwrap_or_default();
            let _ = events_tx.send(crate::server::ServerEvent::SessionCreated {
                session_id: sid.clone(),
                title: mission_session_title(mission),
                creator: "mission".into(),
                project: Some(evt_cwd.clone()),
                project_name: std::path::Path::new(&evt_cwd)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string()),
                skill: None,
                mission_id: Some(mission.id.clone()),
            });
        }
    }
    // Begin a run record
    let run_id = match manager
        .begin_agent_run(
            &root,
            session_id.as_deref(),
            agent_id,
            None,
            Some(format!("mission:{}", mission.id)),
        )
        .await
    {
        Ok(id) => id,
        Err(e) => {
            warn!(
                "Mission scheduler: failed to begin run: {}",
                e
            );
            return;
        }
    };

    // The mission body is injected into the system prompt via active_mission
    // (below). The user turn is a short kickoff so the agent starts executing
    // against the instructions it already has in context.
    let message = format!(
        "Run the \"{}\" mission now per the instructions in your system prompt. Report results in your final message.",
        mission.name.clone().unwrap_or_else(|| mission.id.clone())
    );

    // Persist the mission prompt as a "user" message so it appears in the session chat.
    // Skip if the trigger API already persisted it (pre-created session).
    if !has_pre_session {
        let global_store = crate::state_fs::SessionStore::with_sessions_dir(
            crate::paths::global_sessions_dir(),
        );
        if let Some(sid) = session_id.as_deref() {
            let _ = global_store.add_chat_message(
                sid,
                &crate::state_fs::sessions::ChatMsg {
                    agent_id: agent_id.to_string(),
                    from_id: "user".to_string(),
                    to_id: agent_id.to_string(),
                    content: message.clone(),
                    timestamp: crate::util::now_ts_secs(),
                    is_observation: false,
                },
            );
        }
    }

    // Emit MissionTriggered — session_id is carried directly on the event.
    let _ = state.events_tx.send(ServerEvent::MissionTriggered {
        mission_id: mission.id.clone(),
        agent_id: agent_id.to_string(),
        project_root: project_path.to_string(),
        session_id: session_id.clone(),
    });

    state
        .send_agent_status(
            agent_id.to_string(),
            AgentStatusKind::Working,
            Some("Processing mission".to_string()),
            None,
            session_id.clone(),
        )
        .await;

    engine.observations.clear();
    engine.task = Some(message.clone());
    engine.set_parent_agent(None);
    engine.set_run_id(Some(run_id.clone()));

    // Apply the mission's configured model (frontmatter `model:` field) so
    // missions run on the model the user chose in mission settings. Without
    // this, the engine keeps whatever model_id it was last set to — usually
    // the global default (e.g. gpt-5.5) — ignoring the per-mission choice.
    // Falls back to default when the configured id isn't registered.
    match mission.model.as_deref() {
        Some(mid) if engine.model_manager.has_model(mid) => {
            engine.model_id = mid.to_string();
        }
        Some(mid) => {
            warn!(
                "Mission '{}' requested model '{}' which is not configured — falling back to default '{}'",
                mission.id, mid, engine.default_model_id
            );
            engine.model_id = engine.default_model_id.clone();
        }
        None => {
            engine.model_id = engine.default_model_id.clone();
        }
    }

    // Inject the mission body into the system prompt so the agent reads it as
    // instructions (not as a user turn). Matches how skill bodies are injected
    // via active_skill — see engine/prompt.rs.
    engine.active_mission = Some(crate::engine::ActiveMission {
        name: mission.name.clone().unwrap_or_else(|| mission.id.clone()),
        description: mission.description.clone(),
        body: mission.prompt.clone(),
        mission_dir: Some(state.manager.missions.mission_dir(&mission.id)),
    });
    // Force Auto permission mode (legacy — kept for backward compat with old check flow).
    engine.cfg.tool_permission_mode = crate::config::ToolPermissionMode::Auto;

    // New permission model: apply session policy + path-mode grants.
    //
    // - Policy ("autonomy") decides what happens when the agent tries
    //   something outside its grants:
    //     strict  → silently deny (safe default for unattended runs)
    //     trusted → silently allow (legacy locked-mission behavior)
    //     interactive → prompt (rare for missions — nothing to click)
    // - Path-mode grants come from (a) the mission's permission.mode on
    //   cwd + declared paths, and (b) if a skill is bound to the session,
    //   the skill's declared permission.paths.
    {
        // Missions never prompt — they pause/fail on permission-needed.
        engine.session_permissions.interactive = false;

        // Per-path grants from the mission's permission block. No implicit
        // cwd grant — mission authors list cwd in `permission.paths` if they
        // want it granted (matches SkillPermission semantics).
        if let Some(ref perm) = mission.permission {
            for (path, mode) in perm.iter_grants() {
                engine.session_permissions.set_path_mode(path, mode);
            }
        }

        // If the session binds a skill, apply its declared permission grants.
        // These are narrower than the tier grant (e.g. write on ~/.linggen)
        // and win via longest-path-match in effective_mode_for_path.
        if let Some(ref sid) = session_id {
            if let Ok(Some(meta)) = state.manager.global_sessions.get_session_meta(sid) {
                if let Some(ref skill_name) = meta.skill {
                    if let Some(skill) = state.skill_manager.get_skill(skill_name).await {
                        if let Some(ref perm) = skill.permission {
                            for (path, mode) in perm.iter_grants() {
                                engine
                                    .session_permissions
                                    .set_path_mode(path, mode);
                            }
                        }
                    }
                }
            }
        }

        // Persist so the UI shows the correct mode if user opens the mission session.
        if let Some(ref sid) = session_id {
            let sdir = crate::paths::global_sessions_dir().join(sid);
            engine.session_permissions.save(&sdir);
        }
    }

    // Apply allowed-tools and allow-skills from frontmatter.
    apply_mission_tool_scope(&mut engine, mission);

    // Wire up thinking channel so tokens are emitted as SSE events,
    // allowing the UI to stream mission output in real time.
    let (thinking_tx, mut thinking_rx) =
        tokio::sync::mpsc::unbounded_channel::<crate::engine::ThinkingEvent>();
    engine.thinking_tx = Some(thinking_tx);

    let events_tx_stream = events_tx.clone();
    let agent_id_stream = agent_id.to_string();
    let session_id_stream = session_id.clone();
    tokio::spawn(async move {
        while let Some(event) = thinking_rx.recv().await {
            let (token, done, thinking) = match event {
                crate::engine::ThinkingEvent::Token(t) => (t, false, true),
                crate::engine::ThinkingEvent::ContentToken(t) => (t, false, false),
                crate::engine::ThinkingEvent::Done => (String::new(), true, true),
                crate::engine::ThinkingEvent::ContentDone => (String::new(), true, false),
            };
            let _ = events_tx_stream.send(ServerEvent::Token {
                agent_id: agent_id_stream.clone(),
                token,
                done,
                thinking,
                session_id: session_id_stream.clone(),
            });
            // Emit StateUpdated on content done so the UI reloads persisted messages
            if done && !thinking {
                let _ = events_tx_stream.send(ServerEvent::StateUpdated);
            }
        }
    });

    let result = engine.run_agent_loop(session_id.as_deref()).await;
    engine.thinking_tx = None;
    engine.set_run_id(None);

    let status = match result {
        Ok(outcome) => {
            let _ = manager
                .finish_agent_run(
                    &run_id,
                    crate::agent_manager::AgentRunStatus::Completed,
                    None,
                )
                .await;
            let _ = events_tx.send(ServerEvent::Outcome {
                agent_id: agent_id.to_string(),
                outcome,
                session_id: session_id.clone(),
            });
            "completed"
        }
        Err(err) => {
            let msg = err.to_string();
            let cancelled = msg.to_lowercase().contains("cancel");
            let run_status = if cancelled {
                crate::agent_manager::AgentRunStatus::Cancelled
            } else {
                warn!(
                    "Mission '{}' agent loop failed (run_id={}, session={}): {}",
                    mission.id,
                    mission_run_id,
                    session_id.as_deref().unwrap_or("-"),
                    msg
                );
                crate::agent_manager::AgentRunStatus::Failed
            };
            // Surface the engine error inside the session transcript so the
            // user sees *why* the mission failed — not just a red toast. The
            // "Error:" prefix triggers the UI's isError rendering path
            // (chatStore.ts detects it on both live and persisted messages).
            if !cancelled {
                if let Some(ref sid) = session_id {
                    let _ = state.manager.global_sessions.add_chat_message(
                        sid,
                        &crate::state_fs::sessions::ChatMsg {
                            agent_id: agent_id.to_string(),
                            from_id: agent_id.to_string(),
                            to_id: "user".to_string(),
                            content: format!("Error: {}", msg),
                            timestamp: crate::util::now_ts_secs(),
                            is_observation: false,
                        },
                    );
                    // Ping the UI so it reloads persisted messages immediately
                    // instead of waiting for the next 5s poll.
                    let _ = state.events_tx.send(ServerEvent::StateUpdated);
                }
            }
            let _ = manager
                .finish_agent_run(&run_id, run_status, Some(msg))
                .await;
            "failed"
        }
    };

    state
        .send_agent_status(
            agent_id.to_string(),
            AgentStatusKind::Idle,
            Some("Idle".to_string()),
            None,
            session_id.clone(),
        )
        .await;

    manager
        .update_agent_activity(project_path, agent_id)
        .await;

    // Record mission run. Uses the mission-level run id so MissionRunEntry
    // stays aligned with the per-run output dir and env vars.
    record_mission_run_full(
        &state,
        mission,
        &mission_run_id,
        session_id.as_deref(),
        status,
        false,
        entry_exit_code,
        Some(output_dir.to_string_lossy().into_owned()),
    );

    // Notify UI that the mission finished.
    let _ = state.events_tx.send(ServerEvent::Notification(
        crate::server::NotificationPayload::MissionCompleted {
            mission_id: mission.id.clone(),
            mission_name: mission.name.clone().unwrap_or_else(|| mission.id.clone()),
            status: status.to_string(),
            run_id: mission_run_id.clone(),
            session_id: session_id.clone(),
        },
    ));
}

/// Apply the mission's `allowed-tools` and `allow-skills` to the engine.
///
/// Pure computation in `compute_mission_tool_scope` so it's unit-testable;
/// this wrapper mutates the engine.
fn apply_mission_tool_scope(engine: &mut crate::engine::AgentEngine, mission: &Mission) {
    let scope = compute_mission_tool_scope(&mission.allowed_tools, &mission.allow_skills);
    engine.cfg.mission_allowed_tools = scope.mission_allowed_tools;
    engine.cfg.consumer_allowed_skills = scope.consumer_allowed_skills;
    engine.cfg.bash_allow_prefixes = None; // frontmatter controls bash, not tier
}

/// Resolved tool scope derived from frontmatter.
#[derive(Debug, PartialEq)]
struct MissionToolScope {
    mission_allowed_tools: Option<std::collections::HashSet<String>>,
    consumer_allowed_skills: Option<std::collections::HashSet<String>>,
}

/// Compute the effective tool + skill restrictions for a mission.
///
/// - `allowed-tools` becomes the explicit tool allowlist. Empty → unrestricted.
/// - `allow-skills`:
///     - `[]` (empty) — `Skill` tool not added; skills unreachable.
///     - `["*"]` — `Skill` tool available, no whitelist gate.
///     - `[name, …]` — `Skill` tool available + `consumer_allowed_skills` gate.
///
/// The whitelist gate is independent of `allowed-tools`: a mission with
/// concrete `allow-skills` always sets `consumer_allowed_skills` so the Skill
/// tool cannot invoke unlisted skills, even when `allowed-tools` is empty
/// (unrestricted). Otherwise `allow-skills: [memory]` with no `allowed-tools`
/// would silently allow any skill — a real hazard.
fn compute_mission_tool_scope(
    allowed_tools: &[String],
    allow_skills: &[String],
) -> MissionToolScope {
    use std::collections::HashSet;

    let star = allow_skills.iter().any(|s| s == "*");
    let has_concrete_skills = !allow_skills.is_empty() && !star;

    let mut mission_allowed_tools: Option<HashSet<String>> =
        if allowed_tools.is_empty() {
            None
        } else {
            Some(allowed_tools.iter().cloned().collect())
        };

    // Ensure `Skill` is in the allowlist when the mission declares any
    // skills — but only if an allowlist exists. An empty `allowed_tools`
    // means unrestricted, so `Skill` is already implicitly available.
    if (has_concrete_skills || star) && mission_allowed_tools.is_some() {
        if let Some(ref mut set) = mission_allowed_tools {
            set.insert("Skill".to_string());
        }
    }

    let consumer_allowed_skills = if has_concrete_skills {
        Some(allow_skills.iter().cloned().collect())
    } else {
        None
    };

    MissionToolScope {
        mission_allowed_tools,
        consumer_allowed_skills,
    }
}

fn record_mission_run(
    state: &Arc<ServerState>,
    mission: &Mission,
    run_id: &str,
    session_id: Option<&str>,
    status: &str,
    skipped: bool,
) {
    record_mission_run_full(state, mission, run_id, session_id, status, skipped, None, None);
}

#[allow(clippy::too_many_arguments)]
fn record_mission_run_full(
    state: &Arc<ServerState>,
    mission: &Mission,
    run_id: &str,
    session_id: Option<&str>,
    status: &str,
    skipped: bool,
    entry_exit_code: Option<i32>,
    output_dir: Option<String>,
) {
    let entry = MissionRunEntry {
        run_id: run_id.to_string(),
        session_id: session_id.map(|s| s.to_string()),
        triggered_at: crate::util::now_ts_secs(),
        status: status.to_string(),
        skipped,
        entry_exit_code,
        output_dir,
    };
    let _ = state
        .manager
        .missions
        .append_mission_run(&mission.id, &entry);
}

/// Check `mission.requires` against registered skill capabilities. Returns
/// the name of the first missing capability, or `None` if everything resolves.
async fn find_missing_requires(
    state: &Arc<ServerState>,
    mission: &Mission,
) -> Option<String> {
    if mission.requires.is_empty() {
        return None;
    }
    let all_skills = state.skill_manager.list_skills().await;
    for cap in &mission.requires {
        let resolved = all_skills.iter().any(|s| {
            s.provides
                .as_ref()
                .map(|p| p.iter().any(|c| c == cap))
                .unwrap_or(false)
        });
        if !resolved {
            return Some(cap.clone());
        }
    }
    None
}

/// Run the mission's entry script, pulling mission_dir and last_run_at from
/// state. Thin wrapper around `execute_entry_script`.
async fn run_entry_script(
    mission: &Mission,
    entry: &str,
    output_dir: &std::path::Path,
    mission_run_id: &str,
    state: &Arc<ServerState>,
) -> anyhow::Result<i32> {
    let mission_dir = state.manager.missions.mission_dir(&mission.id);
    let last_run_at = state
        .manager
        .missions
        .last_successful_run_at(&mission.id)
        .map(|t| t.to_string())
        .unwrap_or_default();

    let cwd_str = mission
        .cwd
        .clone()
        .or_else(|| mission.project.clone())
        .unwrap_or_else(|| mission_dir.to_string_lossy().into_owned());
    let cwd_resolved = crate::util::resolve_path(std::path::Path::new(&cwd_str));
    let cwd = if cwd_resolved.is_dir() {
        cwd_resolved
    } else {
        mission_dir.clone()
    };

    execute_entry_script(
        entry,
        &mission.id,
        &mission_dir,
        &cwd,
        output_dir,
        mission_run_id,
        &last_run_at,
    )
    .await
}

/// Pure entry-script runner. No shared state; takes explicit paths so it's
/// unit-testable. Captures stdout/stderr to `output_dir/{stdout,stderr}.log`
/// and returns the exit code. See doc/mission-spec.md → Entry script contract.
async fn execute_entry_script(
    entry: &str,
    mission_id: &str,
    mission_dir: &std::path::Path,
    cwd: &std::path::Path,
    output_dir: &std::path::Path,
    mission_run_id: &str,
    last_run_at: &str,
) -> anyhow::Result<i32> {
    use std::process::Stdio;

    let mission_dir_abs = mission_dir
        .canonicalize()
        .unwrap_or_else(|_| mission_dir.to_path_buf());

    // If `entry` resolves to a real file inside the mission dir, run it directly.
    // Otherwise treat entry as an inline bash command.
    let script_candidate = mission_dir.join(entry);

    let mut cmd = tokio::process::Command::new("bash");
    cmd.current_dir(cwd)
        .env("MISSION_ID", mission_id)
        .env("MISSION_DIR", &mission_dir_abs)
        .env("MISSION_CWD", cwd)
        .env("MISSION_OUTPUT_DIR", output_dir)
        .env("MISSION_LAST_RUN_AT", last_run_at)
        .env("MISSION_RUN_ID", mission_run_id)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if script_candidate.is_file() {
        cmd.arg(&script_candidate);
    } else {
        cmd.arg("-c").arg(entry);
    }

    info!(
        "Mission scheduler: running entry for '{}' (output_dir={})",
        mission_id,
        output_dir.display()
    );
    let output = cmd.output().await?;

    // One-shot writes (open + write-all + close). Previously this used
    // tokio::fs::File + write_all without an explicit flush; tokio's File
    // buffers and does not flush on drop, so a reader (or the next mission
    // step) could observe an empty/partial log. tokio::fs::write closes the
    // file before returning, eliminating the race.
    tokio::fs::write(output_dir.join("stdout.log"), &output.stdout).await?;
    tokio::fs::write(output_dir.join("stderr.log"), &output.stderr).await?;

    let code = output.status.code().unwrap_or(-1);
    if code != 0 {
        let stderr_str = String::from_utf8_lossy(&output.stderr);
        warn!(
            "Mission scheduler: entry script for '{}' exited {}: {}",
            mission_id,
            code,
            stderr_str.trim()
        );
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempMission {
        mission_dir: std::path::PathBuf,
        output_dir: std::path::PathBuf,
        _tmp: tempfile::TempDir,
    }

    fn fresh_mission(id: &str) -> TempMission {
        let tmp = tempfile::tempdir().unwrap();
        let mission_dir = tmp.path().join(id);
        let output_dir = mission_dir.join("runs").join("r1");
        std::fs::create_dir_all(&output_dir).unwrap();
        TempMission { mission_dir, output_dir, _tmp: tmp }
    }

    #[tokio::test]
    async fn entry_inline_command_runs_and_captures_output() {
        let m = fresh_mission("inline");
        let code = execute_entry_script(
            "echo hello from entry && echo err >&2",
            "inline",
            &m.mission_dir,
            &m.mission_dir,
            &m.output_dir,
            "run-1",
            "0",
        )
        .await
        .unwrap();
        assert_eq!(code, 0);

        let stdout = std::fs::read_to_string(m.output_dir.join("stdout.log")).unwrap();
        assert!(stdout.contains("hello from entry"));
        let stderr = std::fs::read_to_string(m.output_dir.join("stderr.log")).unwrap();
        assert!(stderr.contains("err"));
    }

    #[tokio::test]
    async fn entry_nonzero_exit_is_returned() {
        let m = fresh_mission("fail");
        let code = execute_entry_script(
            "exit 7",
            "fail",
            &m.mission_dir,
            &m.mission_dir,
            &m.output_dir,
            "run-2",
            "0",
        )
        .await
        .unwrap();
        assert_eq!(code, 7);
    }

    #[tokio::test]
    async fn entry_env_vars_are_set() {
        let m = fresh_mission("envcheck");
        // Script writes MISSION_* env vars so we can assert they're set.
        let cmd = r#"printf '%s\n%s\n%s\n%s\n%s\n%s\n' \
            "$MISSION_ID" "$MISSION_CWD" "$MISSION_OUTPUT_DIR" \
            "$MISSION_RUN_ID" "$MISSION_LAST_RUN_AT" "$MISSION_DIR""#;
        let code = execute_entry_script(
            cmd,
            "envcheck",
            &m.mission_dir,
            &m.mission_dir,
            &m.output_dir,
            "run-42",
            "1700000000",
        )
        .await
        .unwrap();
        assert_eq!(code, 0);

        let stdout = std::fs::read_to_string(m.output_dir.join("stdout.log")).unwrap();
        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(lines[0], "envcheck");
        // cwd/output_dir strings come back resolved; just check they're non-empty.
        assert!(!lines[1].is_empty());
        assert!(!lines[2].is_empty());
        assert_eq!(lines[3], "run-42");
        assert_eq!(lines[4], "1700000000");
        assert!(!lines[5].is_empty());
    }

    #[tokio::test]
    async fn entry_resolves_relative_script_file() {
        let m = fresh_mission("scripted");
        let scripts = m.mission_dir.join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        let script = scripts.join("hi.sh");
        std::fs::write(&script, "#!/usr/bin/env bash\necho ran-$MISSION_RUN_ID\n").unwrap();

        let code = execute_entry_script(
            "scripts/hi.sh",
            "scripted",
            &m.mission_dir,
            &m.mission_dir,
            &m.output_dir,
            "abc",
            "0",
        )
        .await
        .unwrap();
        assert_eq!(code, 0);

        let stdout = std::fs::read_to_string(m.output_dir.join("stdout.log")).unwrap();
        assert!(stdout.contains("ran-abc"), "got: {}", stdout);
    }

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn tool_scope_empty_means_unrestricted() {
        let s = compute_mission_tool_scope(&[], &[]);
        assert!(s.mission_allowed_tools.is_none(), "empty allowed_tools → no restriction");
        assert!(s.consumer_allowed_skills.is_none(), "no allow-skills → no gate");
    }

    #[test]
    fn tool_scope_star_adds_skill_no_whitelist() {
        let s = compute_mission_tool_scope(&v(&["Read", "Bash"]), &v(&["*"]));
        let set = s.mission_allowed_tools.unwrap();
        assert!(set.contains("Skill"), "star should add Skill to allowlist");
        assert!(set.contains("Read"));
        assert!(s.consumer_allowed_skills.is_none(), "star disables whitelist gate");
    }

    #[test]
    fn tool_scope_concrete_skills_gate_and_add_skill() {
        let s = compute_mission_tool_scope(&v(&["Read"]), &v(&["memory", "linggen"]));
        let set = s.mission_allowed_tools.unwrap();
        assert!(set.contains("Skill"));
        assert!(set.contains("Read"));
        let gate = s.consumer_allowed_skills.unwrap();
        assert!(gate.contains("memory"));
        assert!(gate.contains("linggen"));
    }

    #[test]
    fn tool_scope_concrete_skills_gate_without_tool_list() {
        // Regression: when allowed_tools is empty (unrestricted), a concrete
        // allow-skills list still gates the Skill tool. Previously this path
        // silently skipped the gate, allowing any skill to be called.
        let s = compute_mission_tool_scope(&[], &v(&["memory"]));
        assert!(s.mission_allowed_tools.is_none(), "still unrestricted");
        let gate = s.consumer_allowed_skills.expect("gate must still apply");
        assert!(gate.contains("memory"));
        assert_eq!(gate.len(), 1);
    }

    #[test]
    fn tool_scope_empty_allow_skills_no_skill_tool() {
        // allow-skills: [] means Skill tool stays out of the allowlist.
        let s = compute_mission_tool_scope(&v(&["Read", "Bash"]), &[]);
        let set = s.mission_allowed_tools.unwrap();
        assert!(!set.contains("Skill"), "empty allow-skills → no Skill tool");
        assert!(s.consumer_allowed_skills.is_none());
    }
}


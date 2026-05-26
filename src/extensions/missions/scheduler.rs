use super::{Mission, MissionRunEntry};
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
                .update_agent_activity(&project_path, &mission.agent_id)
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
    let cron_schedule = match super::parse_cron(schedule) {
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

/// User-side "go" message for a mission run.
///
/// The mission body is the full system prompt (injected via `ActiveMission`);
/// the kickoff just gives the agent loop a user turn to react to. Kept short
/// on purpose — output shape, tools, and step-by-step instructions all live
/// in the body, so we don't repeat them here. Shared between the cron path
/// (`dispatch_mission_prompt`) and the manual-trigger API
/// (`api::missions::trigger_mission`).
pub fn mission_kickoff_message(mission: &Mission) -> String {
    let label = mission.name.as_deref().unwrap_or(&mission.id);
    format!("Run the \"{}\" mission per your system prompt.", label)
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
        // Mission sessions carry the mission name as canonical title.
        title_locked: true,
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

/// Turn-seam catch-up. Called from the post-turn seam (owner sessions only).
/// Scans all enabled missions whose `catchup_hours` is set, and fires any
/// whose last non-skipped run is older than that threshold (or which has
/// never run). Used to recover from missed cron fires when the machine was
/// off/asleep — the user's next turn re-triggers the work opportunistically.
///
/// Per mission, opt in by setting `catchup_hours: <n>` in the mission's
/// frontmatter. Omit the field to leave the mission cron-only.
///
/// Cheap + non-blocking: spawns once per call, walks missions, calls
/// `dispatch_mission_prompt_public` for each overdue one. Overlap with the
/// regular cron fire is prevented by the generic mission busy-skip in the
/// scheduler tick.
pub(crate) fn maybe_fire_catchup_missions(state: Arc<ServerState>) {
    tokio::spawn(async move {
        let missions = match state.manager.missions.list_enabled_missions() {
            Ok(m) => m,
            Err(e) => {
                debug!("catchup: list_enabled_missions failed: {e}");
                return;
            }
        };

        let now = crate::util::now_ts_secs();

        for mission in missions {
            let Some(catchup_hours) = mission.catchup_hours else {
                continue;
            };
            if catchup_hours == 0 {
                // 0 would re-fire every turn — treat as opt-out.
                continue;
            }

            // Last *attempt* (completed or failed; skipped doesn't count).
            // None ⇒ never run ⇒ overdue.
            let last_run_secs = match state.manager.missions.list_mission_runs(&mission.id) {
                Ok(runs) => runs
                    .iter()
                    .filter(|r| !r.skipped)
                    .map(|r| r.triggered_at)
                    .max(),
                Err(e) => {
                    debug!("catchup: list_mission_runs failed for '{}': {e}", mission.id);
                    continue;
                }
            };

            let threshold_secs = catchup_hours.saturating_mul(3600);
            let overdue = match last_run_secs {
                None => true,
                Some(last) => now.saturating_sub(last) >= threshold_secs,
            };
            if !overdue {
                continue;
            }

            info!(
                "catchup: mission '{}' last run {:?}s ago (threshold {}h) — triggering",
                mission.id,
                last_run_secs.map(|l| now.saturating_sub(l)),
                catchup_hours
            );
            let (root, project_path) = mission_root(&mission);
            dispatch_mission_prompt_public(state.clone(), root, &project_path, &mission, None)
                .await;
        }
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

    let agent_id = mission.agent_id.as_str();

    // Mission-level run id. Used for the per-run output dir, the
    // MISSION_RUN_ID env var, and the MissionRunEntry.run_id.
    let mission_run_id = format!(
        "mission-run-{}-{}",
        crate::util::now_ts_secs(),
        &uuid::Uuid::new_v4().to_string()[..8]
    );

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
    // against the instructions it already has in context — see
    // `mission_kickoff_message` for why this is short.
    let message = mission_kickoff_message(mission);

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
    // Mission sessions don't write to the user's biographical memory and
    // shouldn't see the core block + memory protocol in their system prompt.
    // Mirrors the chat-handler gate for skill-creator sessions. Invalidate
    // the cached prompt so the next build excludes the memory sections.
    engine.prompt_profile.include_memory = false;
    engine.cached_system_prompt = None;
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
        // want it granted (matches the skill permission semantics).
        if let Some(ref perm) = mission.permission {
            crate::engine::permission::apply_grants(perm, &mut engine.session_permissions);
        }

        // If the session binds a skill, apply its declared permission grants.
        // These are narrower than the tier grant (e.g. write on ~/.linggen)
        // and win via longest-path-match in effective_mode_for_path.
        if let Some(ref sid) = session_id {
            if let Ok(Some(meta)) = state.manager.global_sessions.get_session_meta(sid) {
                if let Some(ref skill_name) = meta.skill {
                    if let Some(skill) = state.skill_manager.get_skill(skill_name).await {
                        if let Some(ref perm) = skill.permission {
                            crate::engine::permission::apply_grants(
                                perm,
                                &mut engine.session_permissions,
                            );
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

/// Apply the mission's `allowed-tools` to the engine.
///
/// Missions and skills are independent: a mission cannot delegate to a skill
/// via the `Skill` tool, and the `Skill` tool is not auto-injected into the
/// allowlist. If a mission omits `allowed-tools`, the engine treats it as
/// "unrestricted" — every built-in tool is callable. Otherwise the listed
/// names are the full set.
///
/// Pure computation in `compute_mission_tool_scope` so it's unit-testable;
/// this wrapper just mutates the engine config.
fn apply_mission_tool_scope(engine: &mut crate::engine::AgentEngine, mission: &Mission) {
    engine.cfg.mission_allowed_tools = compute_mission_tool_scope(&mission.allowed_tools);
    engine.cfg.bash_allow_prefixes = None; // frontmatter controls bash, not tier
}

use crate::extensions::scope::compute_tool_scope as compute_mission_tool_scope;

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
    let invocation = if script_candidate.is_file() {
        crate::extensions::script::Invocation::File(&script_candidate)
    } else {
        crate::extensions::script::Invocation::Inline(entry)
    };

    let env: [(&str, &std::ffi::OsStr); 6] = [
        ("MISSION_ID", mission_id.as_ref()),
        ("MISSION_DIR", mission_dir_abs.as_os_str()),
        ("MISSION_CWD", cwd.as_os_str()),
        ("MISSION_OUTPUT_DIR", output_dir.as_os_str()),
        ("MISSION_LAST_RUN_AT", last_run_at.as_ref()),
        ("MISSION_RUN_ID", mission_run_id.as_ref()),
    ];

    let mut cmd = crate::extensions::script::async_command(invocation, cwd, &env);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

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
        // Empty `allowed-tools` → no restriction. Every built-in is callable.
        assert!(compute_mission_tool_scope(&[]).is_none());
    }

    #[test]
    fn tool_scope_lists_become_allowlist() {
        // Explicit list → that's the full set.
        let set = compute_mission_tool_scope(&v(&["Read", "Bash"])).unwrap();
        assert!(set.contains("Read"));
        assert!(set.contains("Bash"));
        assert_eq!(set.len(), 2);
        // Skill is not auto-injected — missions don't delegate to skills.
        assert!(!set.contains("Skill"));
    }
}


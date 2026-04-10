use crate::project_store::missions::{self, Mission, MissionRunEntry, MISSION_AGENT_ID};
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

            // Missions require the "mission" skill.
            if state.skill_manager.get_skill("mission").await.is_none() {
                warn!("Mission scheduler: \"mission\" skill not installed, skipping '{}'. Run `ling init` to install.", mission.id);
                continue;
            }

            // Determine project root for this mission
            let project_path = mission
                .project
                .clone()
                .unwrap_or_else(|| {
                    std::env::current_dir()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                });
            let root = std::path::PathBuf::from(&project_path);

            // Busy-skip: if previous run is still executing, skip and log.
            if ms.running.load(std::sync::atomic::Ordering::Relaxed) {
                info!(
                    "Mission scheduler: mission '{}' still running, skipping trigger",
                    mission.id
                );
                record_mission_run(&state, mission, "", None, "skipped", true);
                ms.last_fire_minute = Some(current_minute);
                continue;
            }

            // Fire!
            ms.last_fire_minute = Some(current_minute);
            ms.daily_count += 1;
            ms.running.store(true, std::sync::atomic::Ordering::Relaxed);

            info!(
                "Mission scheduler: triggering mission '{}' (project: {:?})",
                mission.id, mission.project
            );

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

/// Create a session title from the mission prompt.
fn mission_session_title(mission: &Mission) -> String {
    let prompt_preview: String = mission.prompt.chars().take(50).collect();
    let suffix = if mission.prompt.chars().count() > 50 {
        "..."
    } else {
        ""
    };
    let time = Local::now().format("%Y-%m-%d %H:%M");
    format!("Mission: {}{} — {}", prompt_preview, suffix, time)
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
    let meta = crate::state_fs::sessions::SessionMeta {
        id: session_id.clone(),
        title: mission_session_title(mission),
        created_at: crate::util::now_ts_secs(),
        skill: Some("mission".to_string()),
        creator: "mission".into(),
        cwd: mission.project.clone(),
        project: mission.project.clone(),
        project_name: mission.project.as_ref().and_then(|p| {
            std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().to_string())
        }),
        mission_id: Some(mission.id.clone()),
        model_id: None,
        consumer_user_id: None,
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

/// Dispatch a mission prompt to the mission agent.
async fn dispatch_mission_prompt(
    state: Arc<ServerState>,
    root: std::path::PathBuf,
    project_path: &str,
    mission: &Mission,
    pre_session_id: Option<String>,
) {
    use crate::server::AgentStatusKind;

    let agent_id = MISSION_AGENT_ID;

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
            record_mission_run(&state, mission, "", None, "failed", false);
            return;
        }
    };

    let mut engine = agent.lock().await;

    let manager = state.manager.clone();
    let events_tx = state.events_tx.clone();

    // Emit session_created so the unified session list updates in real-time
    if !has_pre_session {
        if let Some(ref sid) = session_id {
            let _ = events_tx.send(crate::server::ServerEvent::SessionCreated {
                session_id: sid.clone(),
                title: mission_session_title(mission),
                creator: "mission".into(),
                project: Some(mission.project.clone().unwrap_or_default()),
                project_name: std::path::Path::new(&mission.project.clone().unwrap_or_default())
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string()),
                skill: Some("mission".to_string()),
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

    // Construct the mission message
    let message = format!(
        "[Mission: {}]\n\n{}",
        mission.id, mission.prompt
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
    // Force Auto permission mode (legacy — kept for backward compat with old check flow).
    engine.cfg.tool_permission_mode = crate::config::ToolPermissionMode::Auto;

    // New permission model: lock session + set mode from tier.
    // Locked = no prompts; actions within mode ceiling pass, everything else blocked.
    {
        use crate::engine::permission::PermissionMode;
        let mode = match mission.permission_tier.as_str() {
            "readonly" => PermissionMode::Read,
            "standard" => PermissionMode::Edit,
            _ => PermissionMode::Admin, // "full" or unknown
        };
        let cwd = mission.project.clone().unwrap_or_else(|| "~/".to_string());
        engine.session_permissions.locked = true;
        engine.session_permissions.set_path_mode(&cwd, mode);
        // Persist so the UI shows the correct mode if user opens the mission session.
        if let Some(ref sid) = session_id {
            let sdir = crate::paths::global_sessions_dir().join(sid);
            engine.session_permissions.save(&sdir);
        }
    }

    // Apply legacy permission tier restrictions (bash prefixes, tool set).
    apply_permission_tier(&mut engine, &mission.permission_tier);

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
                    crate::project_store::AgentRunStatus::Completed,
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
            let run_status = if msg.to_lowercase().contains("cancel") {
                crate::project_store::AgentRunStatus::Cancelled
            } else {
                crate::project_store::AgentRunStatus::Failed
            };
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

    // Record mission run
    record_mission_run(&state, mission, &run_id, session_id.as_deref(), status, false);

    // Notify UI that the mission finished.
    let _ = state.events_tx.send(ServerEvent::Notification(
        crate::server::NotificationPayload::MissionCompleted {
            mission_id: mission.id.clone(),
            mission_name: mission.name.clone().unwrap_or_else(|| mission.id.clone()),
            status: status.to_string(),
            run_id: run_id.clone(),
            session_id: session_id.clone(),
        },
    ));
}

/// Configure engine restrictions based on the mission's permission tier.
///
/// - **readonly**: Only read/search/web tools. No Write, Edit, Bash.
/// - **standard**: All tools, but Bash restricted to build/test/git-read commands.
/// - **full** (default): All tools, no restrictions.
fn apply_permission_tier(engine: &mut crate::engine::AgentEngine, tier: &str) {
    use std::collections::HashSet;

    match tier {
        "readonly" => {
            let allowed: HashSet<String> = [
                "Read", "Glob", "Grep", "WebSearch", "WebFetch", "Task",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect();
            engine.cfg.mission_allowed_tools = Some(allowed);
        }
        "standard" => {
            // All tools allowed, but bash restricted to safe commands.
            let prefixes = vec![
                // Build & test
                "cargo ".to_string(),
                "npm ".to_string(),
                "npx ".to_string(),
                "yarn ".to_string(),
                "pnpm ".to_string(),
                "make".to_string(),
                "pytest".to_string(),
                "python -m pytest".to_string(),
                "python -m unittest".to_string(),
                "go ".to_string(),
                "mvn ".to_string(),
                "gradle ".to_string(),
                // Git read-only
                "git status".to_string(),
                "git log".to_string(),
                "git diff".to_string(),
                "git show".to_string(),
                "git branch".to_string(),
                "git remote".to_string(),
                // Safe read commands
                "ls".to_string(),
                "pwd".to_string(),
                "wc ".to_string(),
                "cat ".to_string(),
                "head ".to_string(),
                "tail ".to_string(),
                "find ".to_string(),
                "which ".to_string(),
                "echo ".to_string(),
                "env".to_string(),
                "printenv".to_string(),
                "uname".to_string(),
                "whoami".to_string(),
                "date".to_string(),
                "df ".to_string(),
                "du ".to_string(),
                "tree ".to_string(),
                "file ".to_string(),
            ];
            engine.cfg.bash_allow_prefixes = Some(prefixes);
        }
        _ => {
            // "full" or unknown: no restrictions
        }
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
    let entry = MissionRunEntry {
        run_id: run_id.to_string(),
        session_id: session_id.map(|s| s.to_string()),
        triggered_at: crate::util::now_ts_secs(),
        status: status.to_string(),
        skipped,
    };
    let _ = state
        .manager
        .missions
        .append_mission_run(&mission.id, &entry);
}

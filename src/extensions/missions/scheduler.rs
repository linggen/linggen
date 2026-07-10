use crate::engine::mission::record::{Mission, MissionRunEntry};
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

/// Maximum catch-up attempts per mission per local day. A mission that
/// keeps *failing* must not burn the day's token budget by re-firing on
/// every turn seam — observed with the dream mission (a dozen ~50k-token
/// sessions in one evening ending in a provider 429). Cron fires and
/// manual triggers are not counted against this cap.
const CATCHUP_MAX_ATTEMPTS_PER_DAY: usize = 3;

/// In-flight guard shared by **every** dispatch path — cron tick,
/// turn-seam catch-up, and the manual trigger API. The tick keeps its own
/// per-mission `running` flag, but catch-up and manual runs used to
/// bypass it entirely, so two dream runs could overlap and double-spend.
/// All paths funnel through `dispatch_mission_prompt`, so the guard
/// lives there.
static IN_FLIGHT: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashSet::new()));

/// RAII claim on a mission id in [`IN_FLIGHT`]. Dropping releases the
/// claim, including on early returns and panics inside the run.
struct InFlightGuard(String);

impl InFlightGuard {
    /// Claim `mission_id`; `None` when a run is already in flight.
    fn claim(mission_id: &str) -> Option<Self> {
        let mut set = IN_FLIGHT.lock().expect("IN_FLIGHT lock poisoned");
        set.insert(mission_id.to_string())
            .then(|| Self(mission_id.to_string()))
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = IN_FLIGHT.lock() {
            set.remove(&self.0);
        }
    }
}

/// True while a run of `mission_id` holds the in-flight claim. The
/// trigger API checks this before pre-creating a session, so a skipped
/// trigger doesn't leave an orphan empty session row behind.
pub(crate) fn mission_in_flight(mission_id: &str) -> bool {
    IN_FLIGHT
        .lock()
        .map(|set| set.contains(mission_id))
        .unwrap_or(false)
}

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
    // A fresh process has no live runs — heal rows left `running` by a
    // dead daemon (hang, crash, restart) so history shows the truth and
    // catch-up sees the slot as unfilled.
    state.manager.missions.mark_running_runs_interrupted();

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
                    None,
                    false,
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

/// User-side "go" messages for a mission run. The first item lands in the
/// session immediately as the initial user turn; any remaining items are
/// drained one-per-assistant-final-reply via `AgentEngine.kickoff_queue`
/// so authors can stage multi-turn onboarding (e.g. greet, then start work)
/// without batching everything into one model reply.
///
/// Missions without an explicit `kickoff:` list fall back to a single
/// generic "run the X mission" line.
pub fn mission_kickoff_messages(
    mission: &Mission,
    day: Option<&str>,
    attended: bool,
) -> Vec<String> {
    if let Some(day) = day {
        // Attended day runs (calendar click — a user is watching) get
        // their own kickoff so review steps that need a reachable user
        // never leak into unattended runs.
        if attended && !mission.kickoff_attended.is_empty() {
            return mission
                .kickoff_attended
                .iter()
                .map(|item| item.replace("$DAY", day))
                .collect();
        }
        if !mission.kickoff_day.is_empty() {
            return mission
                .kickoff_day
                .iter()
                .map(|item| item.replace("$DAY", day))
                .collect();
        }
        let label = mission.name.as_deref().unwrap_or(&mission.id);
        return vec![format!(
            "Run the \"{label}\" mission scoped to the single day {day}, per your system prompt."
        )];
    }
    if !mission.kickoff.is_empty() {
        return mission.kickoff.clone();
    }
    let label = mission.name.as_deref().unwrap_or(&mission.id);
    vec![format!("Run the \"{}\" mission per your system prompt.", label)]
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
        // Pin the mission's agent so engine creation resolves to it no
        // matter which code path (UI routing vs scheduler dispatch)
        // touches the session first.
        agent_id: Some(mission.agent_id.clone()),
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
/// Accepts an optional pre-created `session_id` so the caller can return
/// it immediately, and an optional target `day` (YYYY-MM-DD) that swaps
/// in the mission's day-scoped kickoff (`kickoff-day` frontmatter).
pub async fn dispatch_mission_prompt_public(
    state: Arc<ServerState>,
    root: std::path::PathBuf,
    project_path: &str,
    mission: &Mission,
    session_id: Option<String>,
    day: Option<String>,
    attended: bool,
) {
    dispatch_mission_prompt(state, root, project_path, mission, session_id, day, attended)
        .await;
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
            let runs = match state.manager.missions.list_mission_runs(&mission.id) {
                Ok(runs) => runs,
                Err(e) => {
                    debug!("catchup: list_mission_runs failed for '{}': {e}", mission.id);
                    continue;
                }
            };
            let last_run_secs = runs
                .iter()
                .filter(|r| !r.skipped)
                .map(|r| r.triggered_at)
                .max();

            let threshold_secs = catchup_hours.saturating_mul(3600);
            let overdue = match last_run_secs {
                None => true,
                Some(last) => now.saturating_sub(last) >= threshold_secs,
            };
            if !overdue {
                continue;
            }

            // Retry cap: a mission that keeps failing (each failure resets
            // nothing — the catch-up would otherwise re-fire once the
            // threshold re-elapses, or immediately for sub-day thresholds)
            // gets at most CATCHUP_MAX_ATTEMPTS_PER_DAY attempts per local
            // day before we stop trying until tomorrow.
            let day_start_secs = Local::now()
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .and_then(|n| n.and_local_timezone(Local).earliest())
                .map(|dt| dt.timestamp().max(0) as u64)
                .unwrap_or(0);
            let attempts_today = runs
                .iter()
                .filter(|r| !r.skipped && r.triggered_at >= day_start_secs)
                .count();
            if attempts_today >= CATCHUP_MAX_ATTEMPTS_PER_DAY {
                info!(
                    "catchup: mission '{}' already attempted {}x today — capped until tomorrow",
                    mission.id, attempts_today
                );
                continue;
            }

            info!(
                "catchup: mission '{}' last run {:?}s ago (threshold {}h) — triggering",
                mission.id,
                last_run_secs.map(|l| now.saturating_sub(l)),
                catchup_hours
            );
            let (root, project_path) = mission_root(&mission);
            dispatch_mission_prompt_public(
                state.clone(),
                root,
                &project_path,
                &mission,
                None,
                None,
                false,
            )
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
    day: Option<String>,
    attended: bool,
) {
    use crate::server::AgentStatusKind;

    // Refresh from disk so in-flight `mission.md` edits land on the next
    // run without needing a daemon restart. Falls back to the cached
    // copy if the file is gone or unparseable — better to run stale than
    // to silently no-op a scheduled mission.
    let refreshed = state.manager.missions.reload_one(&mission.id);
    let mission = refreshed.as_ref().unwrap_or(mission);
    let agent_id = mission.agent_id.as_str();

    // One run per mission at a time, across ALL trigger paths (cron,
    // catch-up, manual). Held for the whole run; released on drop.
    let Some(_in_flight) = InFlightGuard::claim(&mission.id) else {
        info!(
            "Mission '{}': a run is already in flight — skipping this trigger",
            mission.id
        );
        // A pre-created session (a manual trigger that raced another
        // start past the API's in-flight check) would linger as an
        // empty orphan row in the session list — remove it.
        if let Some(sid) = pre_session_id.as_deref() {
            let _ = state.manager.global_sessions.remove_session(sid);
            let _ = state.events_tx.send(ServerEvent::StateUpdated);
        }
        let skip_id = format!(
            "mission-run-{}-{}",
            crate::util::now_ts_secs(),
            &uuid::Uuid::new_v4().to_string()[..8]
        );
        record_mission_run(&state, mission, &skip_id, None, "skipped", true);
        return;
    };

    // Mission-level run id. Keys MissionRunEntry.run_id and the
    // MissionTriggered event.
    let mission_run_id = format!(
        "mission-run-{}-{}",
        crate::util::now_ts_secs(),
        &uuid::Uuid::new_v4().to_string()[..8]
    );

    // Use pre-created session or create a new one
    let has_pre_session = pre_session_id.is_some();
    let session_id = pre_session_id.or_else(|| create_mission_session(mission));

    // Record the run as `running` up front, finalized at the end of this
    // function — a hang, crash, or daemon restart must leave a visible
    // row (healed to `interrupted` on the next start), never an
    // invisible zombie (2026-07-10 dream hang left no record at all).
    record_mission_run(
        &state,
        mission,
        &mission_run_id,
        session_id.as_deref(),
        "running",
        false,
    );

    // Memory-agent runs end in the condense stage, whose replace_ids
    // merges retire long-term rows with no undo — snapshot the store
    // first (one export per day, 7 kept, best-effort).
    if mission.agent_id == "memory" {
        crate::engine::tools::memory_tool::backup_store_best_effort().await;
    }

    let sid = session_id.as_deref().unwrap_or("default");
    let agent = match state.manager.get_or_create_session_agent(sid, &root, agent_id).await {
        Ok(a) => a,
        Err(e) => {
            warn!(
                "Mission scheduler: failed to get mission agent: {}",
                e
            );
            finalize_mission_run(&state, mission, &mission_run_id, "failed");
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
    // (below). The kickoff list seeds the user side of the conversation:
    // item 0 becomes the first user turn that drives the agent loop; the
    // remainder fire one-per-assistant-final-reply via the engine's
    // kickoff_queue (see AgentEngine.try_drain_kickoff).
    let kickoff_items = mission_kickoff_messages(mission, day.as_deref(), attended);
    let (first_message, queued) = kickoff_items
        .split_first()
        .map(|(first, rest)| (first.clone(), rest.to_vec()))
        .expect("mission_kickoff_messages always returns at least one item");

    // Persist the first kickoff item as a user message in the session.
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
                    content: first_message.clone(),
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
    engine.task = Some(first_message.clone());
    engine.kickoff_queue = queued.into();
    engine.kickoff_stop = mission.kickoff_stop.clone();
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
    let before = engine.prompt_profile.include_memory;
    engine.prompt_profile.include_memory = false;
    engine.cached_system_prompt = None;
    tracing::info!(
        "mission '{}' scheduler: prompt_profile.include_memory {} → false (cache cleared)",
        mission.id,
        before
    );
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
                    if let Some(skill) = state.skills.get_skill(skill_name).await {
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

    // Attended runs have a present user — AskUser joins the mission's
    // tool scope so review questions can be asked. The agent's own spec
    // must also list AskUser (tool scopes intersect); unattended runs
    // keep it out of scope regardless of the agent spec. The AskUser
    // bridge must also be wired by hand: chat sessions get it from
    // wire_engine_bridges, but mission engines never pass through the
    // chat runtime — without it the tool returns "not available"
    // immediately instead of blocking on the user's answer.
    if attended {
        if let Some(ref mut set) = engine.cfg.mission_allowed_tools {
            set.insert("AskUser".to_string());
        }
        engine.tools.set_ask_user_bridge(std::sync::Arc::new(
            crate::engine::tools::AskUserBridge {
                events_tx: state.events_tx.clone(),
                pending: state.pending_ask_user.clone(),
                session_id: session_id.clone(),
            },
        ));
        info!(
            "Mission '{}': attended run — AskUser in scope, bridge wired (session {:?})",
            mission.id, session_id
        );
    }

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
                    crate::engine::agent::AgentRunStatus::Completed,
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
                crate::engine::agent::AgentRunStatus::Cancelled
            } else {
                warn!(
                    "Mission '{}' agent loop failed (run_id={}, session={}): {}",
                    mission.id,
                    mission_run_id,
                    session_id.as_deref().unwrap_or("-"),
                    msg
                );
                crate::engine::agent::AgentRunStatus::Failed
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
            if cancelled { "cancelled" } else { "failed" }
        }
    };

    // Engine-composed run report — mechanical truth from the run's
    // memory tool results, appended after the model's final reply so
    // the session always ends with what actually happened (the model's
    // own status lines are best-effort and have been observed wrong).
    if status == "completed" {
        append_run_report(&state, agent_id, session_id.as_deref()).await;
    }

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

    finalize_mission_run(&state, mission, &mission_run_id, status);

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

/// Append the engine-composed run report (see `super::report`) to the
/// run's session as an agent message, then ping the UI to reload.
async fn append_run_report(state: &Arc<ServerState>, agent_id: &str, session_id: Option<&str>) {
    let Some(sid) = session_id else { return };
    let Ok(messages) = state.manager.global_sessions.get_chat_history(sid) else {
        return;
    };
    let Some(mut report) = super::report::compose_memory_report(&messages) else {
        return;
    };

    // Close with where the store stands — one `stats` call to the
    // daemon; skipped silently if it's unreachable.
    let ling_mem_url = state.manager.get_config_snapshot().await.agent.ling_mem_url;
    if let Ok(stats) = crate::engine::tools::memory_tool::call_memory_http(
        &ling_mem_url,
        "Memory_query",
        serde_json::json!({ "verb": "stats" }),
    )
    .await
    {
        if let Some(line) = super::report::store_state_line(&stats) {
            report.push_str("\n- ");
            report.push_str(&line);
        }
    }
    let _ = state.manager.global_sessions.add_chat_message(
        sid,
        &crate::state_fs::sessions::ChatMsg {
            agent_id: agent_id.to_string(),
            from_id: agent_id.to_string(),
            to_id: "user".to_string(),
            content: report,
            timestamp: crate::util::now_ts_secs(),
            is_observation: false,
        },
    );
    let _ = state.events_tx.send(ServerEvent::StateUpdated);
}

/// Flip the up-front `running` entry to its terminal status. The entry
/// keeps its original `triggered_at` (the actual start time).
fn finalize_mission_run(
    state: &Arc<ServerState>,
    mission: &Mission,
    run_id: &str,
    status: &str,
) {
    if let Err(e) = state
        .manager
        .missions
        .update_mission_run_status(&mission.id, run_id, status)
    {
        warn!(
            "Mission '{}': failed to finalize run {} as {}: {}",
            mission.id, run_id, status, e
        );
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

#[cfg(test)]
mod tests {
    use super::*;

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


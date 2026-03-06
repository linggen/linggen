use crate::project_store::missions::{self, MissionRunEntry, Mission};
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
}

impl MissionState {
    fn new() -> Self {
        Self {
            last_fire_minute: None,
            daily_count: 0,
            daily_date: None,
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
        // Current minute as a dedup key (minutes since epoch)
        let current_minute = now.timestamp() / 60;

        let projects = match state.manager.store.list_projects() {
            Ok(p) => p,
            Err(e) => {
                debug!("Mission scheduler: failed to list projects: {}", e);
                continue;
            }
        };

        for project in &projects {
            let project_path = &project.path;

            // Run migration on first access (idempotent)
            let _ = state.manager.store.migrate_old_missions(project_path);

            let enabled_missions = match state.manager.store.list_enabled_missions(project_path) {
                Ok(m) => m,
                Err(_) => continue,
            };

            for mission in &enabled_missions {
                let mission_key = format!("{}|{}", project_path, mission.id);

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

                // Check if agent is busy
                let root = std::path::PathBuf::from(project_path);
                let agent = match state
                    .manager
                    .get_or_create_agent(&root, &mission.agent_id)
                    .await
                {
                    Ok(a) => a,
                    Err(_) => continue,
                };

                if agent.try_lock().is_err() {
                    debug!(
                        "Mission scheduler: agent '{}' is busy, skipping mission '{}'",
                        mission.agent_id, mission.id
                    );
                    // Log skipped trigger
                    let entry = MissionRunEntry {
                        run_id: String::new(),
                        triggered_at: crate::util::now_ts_secs(),
                        status: "skipped".to_string(),
                        skipped: true,
                    };
                    let _ = state
                        .manager
                        .store
                        .append_mission_run(project_path, &mission.id, &entry);
                    continue;
                }

                // Fire!
                ms.last_fire_minute = Some(current_minute);
                ms.daily_count += 1;

                info!(
                    "Mission scheduler: triggering mission '{}' for agent '{}' in project '{}'",
                    mission.id, mission.agent_id, project_path
                );

                let _ = state.events_tx.send(ServerEvent::MissionTriggered {
                    mission_id: mission.id.clone(),
                    agent_id: mission.agent_id.clone(),
                    project_root: project_path.clone(),
                });

                state
                    .manager
                    .update_agent_activity(project_path, &mission.agent_id)
                    .await;

                let state_clone = state.clone();
                let mission_owned = mission.clone();
                let project_path_owned = project_path.clone();
                let root_owned = root.clone();

                tokio::spawn(async move {
                    dispatch_mission_prompt(
                        state_clone,
                        root_owned,
                        &project_path_owned,
                        &mission_owned,
                    )
                    .await;
                });
            }
        }

        // Clean up state for missions that no longer exist
        let active_keys: std::collections::HashSet<String> = projects
            .iter()
            .flat_map(|p| {
                state
                    .manager
                    .store
                    .list_enabled_missions(&p.path)
                    .unwrap_or_default()
                    .into_iter()
                    .map(move |m| format!("{}|{}", p.path, m.id))
            })
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

    // Get the next upcoming occurrence after one minute ago.
    // If it falls within the current minute, the cron matches now.
    let one_min_ago = *now - chrono::Duration::seconds(60);
    if let Some(next) = cron_schedule.after(&one_min_ago).next() {
        // Check if `next` is within the current minute
        let next_minute = next.timestamp() / 60;
        let now_minute = now.timestamp() / 60;
        next_minute == now_minute
    } else {
        false
    }
}

/// Dispatch a mission prompt to an agent.
async fn dispatch_mission_prompt(
    state: Arc<ServerState>,
    root: std::path::PathBuf,
    project_path: &str,
    mission: &Mission,
) {
    use crate::server::AgentStatusKind;

    let agent_id = &mission.agent_id;

    let agent = match state.manager.get_or_create_agent(&root, agent_id).await {
        Ok(a) => a,
        Err(e) => {
            warn!(
                "Mission scheduler: failed to get agent '{}': {}",
                agent_id, e
            );
            record_mission_run(&state, project_path, mission, "", "failed", false);
            return;
        }
    };

    let Ok(mut engine) = agent.try_lock() else {
        debug!(
            "Mission scheduler: agent '{}' became busy before dispatch",
            agent_id
        );
        record_mission_run(&state, project_path, mission, "", "skipped", true);
        return;
    };

    let manager = state.manager.clone();
    let events_tx = state.events_tx.clone();

    // Begin a run record
    let run_id = match manager
        .begin_agent_run(
            &root,
            None,
            agent_id,
            None,
            Some(format!("mission:{}", mission.id)),
        )
        .await
    {
        Ok(id) => id,
        Err(e) => {
            warn!(
                "Mission scheduler: failed to begin run for '{}': {}",
                agent_id, e
            );
            return;
        }
    };

    // Construct the mission message
    let message = format!(
        "[Mission: {}]\n\n{}",
        mission.id, mission.prompt
    );

    // Persist and emit the mission prompt as a system message
    crate::server::chat_helpers::persist_and_emit_message(
        &manager,
        &events_tx,
        &root,
        agent_id,
        "system",
        agent_id,
        &message,
        None,
        false,
    )
    .await;

    state
        .send_agent_status(
            agent_id.to_string(),
            AgentStatusKind::Working,
            Some("Processing mission".to_string()),
            None,
        )
        .await;

    engine.observations.clear();
    engine.task = Some(message.clone());
    engine.set_parent_agent(None);
    engine.set_run_id(Some(run_id.clone()));
    let result = engine.run_agent_loop(None).await;
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
        )
        .await;

    manager
        .update_agent_activity(project_path, agent_id)
        .await;

    // Record mission run
    record_mission_run(&state, project_path, mission, &run_id, status, false);
}

fn record_mission_run(
    state: &Arc<ServerState>,
    project_path: &str,
    mission: &Mission,
    run_id: &str,
    status: &str,
    skipped: bool,
) {
    let entry = MissionRunEntry {
        run_id: run_id.to_string(),
        triggered_at: crate::util::now_ts_secs(),
        status: status.to_string(),
        skipped,
    };
    let _ = state
        .manager
        .store
        .append_mission_run(project_path, &mission.id, &entry);
}

//! A session's background tasks (doc/background-tasks-spec.md): work an agent
//! started and did not wait for. Both runtimes feed this one list; the subagent
//! pane shows it and stops a task through it. In engine memory only: a task
//! never outlives the engine that ran it.
use crate::engine::events::{BackgroundTask, ServerEvent};
use std::collections::HashMap;
use std::sync::Mutex;

/// Ended tasks a session keeps listed, newest first, beside its running ones.
const KEEP_ENDED: usize = 20;

#[derive(Default)]
pub(crate) struct Tasks {
    by_session: Mutex<HashMap<String, Vec<BackgroundTask>>>,
}

impl Tasks {
    fn with<T>(&self, f: impl FnOnce(&mut HashMap<String, Vec<BackgroundTask>>) -> T) -> T {
        let mut guard = self
            .by_session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut guard)
    }

    /// Sessions with background work still running — the session list's
    /// spinner reads them, cheaply, from memory.
    pub fn sessions_running(&self) -> Vec<String> {
        self.with(|all| {
            all.iter()
                .filter(|(_, tasks)| tasks.iter().any(|t| t.is_running()))
                .map(|(sid, _)| sid.clone())
                .collect()
        })
    }

    /// Running first in start order, then ended ones, newest first.
    pub fn list(&self, session: &str) -> Vec<BackgroundTask> {
        self.with(|all| all.get(session).cloned().unwrap_or_default())
    }

    pub fn get(&self, session: &str, id: &str) -> Option<BackgroundTask> {
        self.with(|all| all.get(session)?.iter().find(|t| t.id == id).cloned())
    }

    /// Add the task, or update the one with its id. False when nothing changed.
    pub fn upsert(&self, session: &str, task: BackgroundTask) -> bool {
        self.with(|all| {
            let tasks = all.entry(session.into()).or_default();
            match tasks.iter_mut().find(|t| t.id == task.id) {
                Some(old) if *old == task => false,
                Some(old) => {
                    *old = task;
                    tidy(tasks);
                    true
                }
                None => {
                    tasks.push(task);
                    tidy(tasks);
                    true
                }
            }
        })
    }

    /// Change one task in place. False when it is not listed.
    pub fn update(&self, session: &str, id: &str, f: impl FnOnce(&mut BackgroundTask)) -> bool {
        self.with(|all| {
            let Some(tasks) = all.get_mut(session) else {
                return false;
            };
            let Some(task) = tasks.iter_mut().find(|t| t.id == id) else {
                return false;
            };
            f(task);
            tidy(tasks);
            true
        })
    }

    pub fn finish(&self, session: &str, id: &str, status: &str, summary: Option<String>) -> bool {
        self.update(session, id, |task| {
            task.end(status);
            if summary.is_some() {
                task.summary = summary;
            }
        })
    }

    /// What ran them is gone (a runner ended, the engine stopped them):
    /// whatever still runs is interrupted. False when nothing ran.
    pub fn interrupt_running(&self, session: &str) -> bool {
        self.with(|all| {
            let Some(tasks) = all.get_mut(session) else {
                return false;
            };
            let mut changed = false;
            for task in tasks.iter_mut().filter(|t| t.is_running()) {
                task.end("interrupted");
                changed = true;
            }
            tidy(tasks);
            changed
        })
    }

    /// Interrupt subagent tasks nothing follows any more. `alive` says
    /// whether a task (with its recorded run) of a session still is.
    pub fn interrupt_stale_subagents(&self, alive: impl Fn(&str, &BackgroundTask) -> bool) -> Vec<String> {
        self.with(|all| {
            let mut changed = Vec::new();
            for (session, tasks) in all.iter_mut() {
                let mut dirty = false;
                for task in tasks.iter_mut().filter(|t| t.is_running() && t.kind == "subagent") {
                    if task.subagent_id.is_some() && !alive(session, task) {
                        task.end("interrupted");
                        dirty = true;
                    }
                }
                if dirty {
                    tidy(tasks);
                    changed.push(session.clone());
                }
            }
            changed
        })
    }

    /// The conversation ended (Clear, delete): its tasks leave the list.
    pub fn forget(&self, session: &str) -> bool {
        self.with(|all| all.remove(session).is_some())
    }
}

fn tidy(tasks: &mut Vec<BackgroundTask>) {
    tasks.sort_by(|a, b| match (a.is_running(), b.is_running()) {
        (true, true) => a.started_at.cmp(&b.started_at),
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => b.ended_at.cmp(&a.ended_at),
    });
    let running = tasks.iter().filter(|t| t.is_running()).count();
    tasks.truncate(running + KEEP_ENDED);
}

#[derive(serde::Deserialize)]
pub(crate) struct StopRequest {
    session_id: String,
    id: String,
}

/// POST /api/tasks/stop — the person stops one background task. Never the
/// turn, never the session's other tasks.
pub(crate) async fn stop(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<super::ServerState>>,
    axum::Json(req): axum::Json<StopRequest>,
) -> axum::response::Response {
    use axum::{http::StatusCode, response::IntoResponse};
    let Some(task) = state.tasks.get(&req.session_id, &req.id) else {
        return (StatusCode::NOT_FOUND, "No such background task.").into_response();
    };
    if !task.is_running() {
        return axum::Json(serde_json::json!({"status": "ok"})).into_response();
    }
    let runtime = state
        .manager
        .global_sessions
        .get_session_meta(&req.session_id)
        .ok()
        .flatten()
        .map(|meta| meta.runtime);
    let stopped = match runtime {
        Some(runtime) if !runtime.is_linggen() => {
            super::claude::stop_task(&state, &req.session_id, &req.id).await
        }
        _ => state.manager.background.stop(&req.session_id, &req.id),
    };
    match stopped {
        Ok(()) => axum::Json(serde_json::json!({"status": "ok"})).into_response(),
        Err(error) => (StatusCode::CONFLICT, error.to_string()).into_response(),
    }
}

/// Native sessions' tasks: each change the engine reports is listed; a
/// task's end is told to the agent that started it, in a turn of its own.
pub(crate) async fn native_watch(state: std::sync::Arc<super::ServerState>) {
    let mut events = state.events_tx.subscribe();
    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        };
        let ServerEvent::BackgroundTask { session_id: Some(session), agent_id, task, notice } = event else {
            continue;
        };
        // A task's end is told once: on the change from running to ended.
        let was_running = state.tasks.get(&session, &task.id).is_none_or(|t| t.is_running());
        let ended = was_running && !task.is_running();
        if state.tasks.upsert(&session, task) {
            emit(&state, &session);
        }
        let (true, Some(notice)) = (ended, notice) else {
            continue;
        };
        // While the agent's plan waits for the person, its task ends wait
        // behind it, as Claude Code holds them behind its plan prompt.
        let Some(notice) = state.manager.hold_behind_plan(&session, &agent_id, notice).await else {
            continue;
        };
        tokio::spawn(tell(state.clone(), session, agent_id, notice));
    }
}

/// Tell `agent` each of `notices` in a turn of its own, in order: their
/// places in the turn line are taken now, one after another.
pub(crate) fn tell_all(
    state: &std::sync::Arc<super::ServerState>,
    session: &str,
    agent: &str,
    notices: Vec<String>,
) {
    for notice in notices {
        tokio::spawn(tell(state.clone(), session.into(), agent.into(), notice));
    }
}

/// Give `agent` a turn in `session` that starts with `notice`, after any
/// turn already running there. Its place in the session's turn line is
/// taken on the call, before the returned task first runs.
pub(crate) fn tell(
    state: std::sync::Arc<super::ServerState>,
    session: String,
    agent: String,
    notice: String,
) -> impl std::future::Future<Output = ()> + Send + 'static {
    tell_from(state, session, agent, notice, "system".into())
}

pub(crate) fn tell_from(
    state: std::sync::Arc<super::ServerState>,
    session: String,
    agent: String,
    notice: String,
    sender: String,
) -> impl std::future::Future<Output = ()> + Send + 'static {
    let (_, place) = super::chat::helpers::take_turn_place(state.manager.session_turn(&session));
    tell_in_place(state, session, agent, notice, sender, place)
}

async fn tell_in_place(
    state: std::sync::Arc<super::ServerState>,
    session: String,
    agent: String,
    notice: String,
    sender: String,
    place: super::chat::helpers::TurnPlace,
) {
    let Some(meta) = state.manager.global_sessions.get_session_meta(&session).ok().flatten() else {
        return; // the session was deleted meanwhile
    };
    if !meta.runtime.is_linggen() {
        return;
    }
    let root = std::path::PathBuf::from(meta.project.clone().unwrap_or_else(|| meta.cwd.clone()));
    let engine = match state.manager.get_or_create_session_agent(&session, &root, &agent).await {
        Ok(engine) => engine,
        Err(error) => {
            tracing::warn!("[tasks] could not wake {agent} in {session}: {error}");
            return;
        }
    };
    let _turn = place.await;
    // A plan proposed while this waited for its turn holds it too.
    let Some(notice) = state.manager.hold_behind_plan(&session, &agent, notice).await else {
        return;
    };
    super::chat::helpers::persist_and_emit_message(
        &state.manager,
        &state.events_tx,
        &root,
        &agent,
        &sender,
        &agent,
        &notice,
        Some(&session),
        false,
        None,
    )
    .await;
    let mut engine = engine.lock().await;
    engine.set_parent_agent(None);
    engine.last_assistant_text = None;
    let ctx = super::chat::ChatRunCtx {
        via: None,
        state: state.clone(),
        manager: state.manager.clone(),
        events_tx: state.events_tx.clone(),
        root,
        agent_id: agent.clone(),
        session_id: Some(session.clone()),
        clean_msg: notice,
        images: Vec::new(),
        policy: crate::engine::session_policy::SessionPolicy::owner(),
        sender: Some(sender),
        silence_ok: false,
    };
    super::chat::run_session_turn(&ctx, &mut engine, &state.manager, None).await;
}

/// Push the session's list to its surfaces.
pub(crate) fn emit(state: &super::ServerState, session: &str) {
    let _ = state.events_tx.send(ServerEvent::TasksUpdated {
        session_id: session.into(),
        tasks: state.tasks.list(session),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_subagent_task_is_interrupted() {
        let tasks = Tasks::default();
        let mut stale = BackgroundTask::running("stale", "subagent", "review");
        stale.subagent_id = Some("ended".into());
        let mut live = BackgroundTask::running("live", "subagent", "review");
        live.subagent_id = Some("running".into());
        tasks.upsert("s", stale);
        tasks.upsert("s", live);
        tasks.upsert("s", BackgroundTask::running("starting", "subagent", "starting"));
        tasks.upsert("s", BackgroundTask::running("shell", "shell", "work"));
        assert_eq!(tasks.interrupt_stale_subagents(|_, t| t.subagent_id.as_deref() == Some("running")), vec!["s"]);
        assert_eq!(tasks.get("s", "stale").unwrap().status, "interrupted");
        assert!(tasks.get("s", "stale").unwrap().ended_at.is_some());
        assert!(tasks.get("s", "live").unwrap().is_running());
        assert!(tasks.get("s", "starting").unwrap().is_running());
        assert!(tasks.get("s", "shell").unwrap().is_running());
        assert!(tasks.interrupt_stale_subagents(|_, t| t.subagent_id.as_deref() == Some("running")).is_empty());
    }

    #[test]
    fn running_tasks_lead_and_ended_ones_follow_newest_first() {
        let tasks = Tasks::default();
        tasks.upsert("s", BackgroundTask::running("a", "shell", "make"));
        tasks.upsert("s", BackgroundTask::running("b", "subagent", "review"));
        tasks.upsert("s", BackgroundTask::running("c", "watch", "tail log"));
        assert!(tasks.finish("s", "a", "completed", Some("built".into())));
        assert!(tasks.finish("s", "c", "failed", None));
        let ids: Vec<_> = tasks.list("s").into_iter().map(|t| t.id).collect();
        assert_eq!(ids[0], "b");
        assert_eq!(tasks.get("s", "a").unwrap().summary.as_deref(), Some("built"));
        assert!(tasks.get("s", "a").unwrap().ended_at.is_some());
        assert!(!tasks.upsert("s", tasks.get("s", "b").unwrap()), "an unchanged task is no change");
    }

    #[test]
    fn a_gone_runner_interrupts_what_still_runs_and_a_clear_forgets_all() {
        let tasks = Tasks::default();
        tasks.upsert("s", BackgroundTask::running("a", "shell", "make"));
        tasks.upsert("s", BackgroundTask::running("b", "shell", "test"));
        tasks.finish("s", "b", "completed", None);
        assert!(tasks.interrupt_running("s"));
        assert_eq!(tasks.get("s", "a").unwrap().status, "interrupted");
        assert_eq!(tasks.get("s", "b").unwrap().status, "completed");
        assert!(!tasks.interrupt_running("s"), "nothing left running");
        assert!(tasks.forget("s"));
        assert!(tasks.list("s").is_empty());
    }

    #[test]
    fn a_session_keeps_only_its_recent_ended_tasks() {
        let tasks = Tasks::default();
        tasks.upsert("s", BackgroundTask::running("live", "shell", "serve"));
        for i in 0..(KEEP_ENDED + 5) {
            let id = format!("t{i}");
            tasks.upsert("s", BackgroundTask::running(&id, "shell", "x"));
            tasks.finish("s", &id, "completed", None);
        }
        let list = tasks.list("s");
        assert_eq!(list.len(), KEEP_ENDED + 1);
        assert_eq!(list[0].id, "live");
    }
}

#[cfg(test)]
mod running_sessions_tests {
    use super::*;

    #[test]
    fn a_session_with_running_background_work_is_listed_until_it_ends() {
        let tasks = Tasks::default();
        tasks.upsert("s", BackgroundTask::running("a", "subagent", "review"));
        tasks.upsert("t", BackgroundTask::running("b", "shell", "make"));
        assert!(tasks.finish("t", "b", "completed", None));
        assert_eq!(tasks.sessions_running(), vec!["s".to_string()]);
        assert!(tasks.finish("s", "a", "completed", None));
        assert!(tasks.sessions_running().is_empty());
    }
}

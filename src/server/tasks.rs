//! A session's background tasks (doc/background-tasks-spec.md): work an agent
//! started and did not wait for. This one list is what the subagent
//! pane shows and stops a task through it. In engine memory only: a task
//! never outlives the engine that ran it.
use crate::engine::events::{BackgroundTask, ServerEvent};
use crate::server::resident::Origin;
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

    /// Interrupt subagent tasks nothing follows any more. `alive` says
    /// whether a task (with its recorded run) of a session still is.
    pub fn interrupt_stale_subagents(
        &self,
        alive: impl Fn(&str, &BackgroundTask) -> bool,
    ) -> Vec<String> {
        self.with(|all| {
            let mut changed = Vec::new();
            for (session, tasks) in all.iter_mut() {
                let mut dirty = false;
                for task in tasks
                    .iter_mut()
                    .filter(|t| t.is_running() && t.kind == "subagent")
                {
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

    /// Tasks listed as running whose work, as `engine` (the engine's own
    /// list) has it, has ended: `(session, the ended task)`.
    pub fn ended_unheard(
        &self,
        engine: impl Fn(&str, &str) -> Option<BackgroundTask>,
    ) -> Vec<(String, BackgroundTask)> {
        self.with(|all| {
            let mut found = Vec::new();
            for (session, tasks) in all.iter() {
                for task in tasks.iter().filter(|t| t.is_running()) {
                    if let Some(ended) = engine(session, &task.id).filter(|t| !t.is_running()) {
                        found.push((session.clone(), ended));
                    }
                }
            }
            found
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
/// turn, never the session's other tasks. A native task is found in the
/// engine's own list too: this list hears of a task through the event bus,
/// which may not have caught up when the turn that started it has ended.
pub(crate) async fn stop(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<super::ServerState>>,
    axum::Json(req): axum::Json<StopRequest>,
) -> axum::response::Response {
    use axum::{http::StatusCode, response::IntoResponse};
    let listed = state.tasks.get(&req.session_id, &req.id);
    let Some(task) = listed.or_else(|| state.manager.background.get(&req.session_id, &req.id)) else {
        return (StatusCode::NOT_FOUND, "No such background task.").into_response();
    };
    if !task.is_running() {
        return axum::Json(serde_json::json!({"status": "ok"})).into_response();
    }
    match state.manager.background.stop(&req.session_id, &req.id) {
        Ok(()) => axum::Json(serde_json::json!({"status": "ok"})).into_response(),
        Err(error) => (StatusCode::CONFLICT, error.to_string()).into_response(),
    }
}

/// Native sessions' tasks: each change the engine reports is listed; a
/// task's end is told to the agent that started it, in a turn of its own.
/// The bus may drop events: when it does, the ends it dropped are read from
/// the engine's own list ([`settle_missed`]).
pub(crate) async fn native_watch(state: std::sync::Arc<super::ServerState>) {
    let mut events = state.events_tx.listen();
    loop {
        let event = match events.recv().await {
            Ok(event) => event,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                tracing::warn!("[tasks] the event bus dropped {missed} events; settling from the engine's list");
                settle_missed(&state).await;
                continue;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        };
        let ServerEvent::BackgroundTask {
            session_id: Some(session),
            task,
            ..
        } = event
        else {
            continue;
        };
        settle(&state, &session, task).await;
    }
}

/// Tasks listed as running whose work has ended in the engine, their end
/// not heard on the bus: each is settled as if it had been. The sweeper runs
/// this too.
pub(crate) async fn settle_missed(state: &std::sync::Arc<super::ServerState>) {
    let missed = state
        .tasks
        .ended_unheard(|session, id| state.manager.background.get(session, id));
    for (session, task) in missed {
        settle(state, &session, task).await;
    }
}

/// List `task`'s change; on its end, tell the agent that started it. The end
/// notice is taken from the engine, once, so an end heard twice (the bus and
/// the sweep) is told once.
async fn settle(state: &std::sync::Arc<super::ServerState>, session: &str, task: BackgroundTask) {
    let was_running = state
        .tasks
        .get(session, &task.id)
        .is_none_or(|t| t.is_running());
    let ended = was_running && !task.is_running();
    let (id, started_in) = (task.id.clone(), task.place.clone());
    if state.tasks.upsert(session, task) {
        emit(state, session);
    }
    if !ended {
        return;
    }
    let Some((agent_id, notice)) = state.manager.background.take_end_notice(session, &id) else {
        return;
    };
    // While the agent's plan waits for the person, its task ends wait
    // behind it, as Claude Code holds them behind its plan prompt.
    let Some(notice) = state
        .manager
        .hold_behind_plan(session, &agent_id, notice)
        .await
    else {
        return;
    };
    let (_, place) = super::chat::helpers::take_turn_place(state.manager.session_turn(session));
    let told = (session.to_string(), agent_id, notice, "system".to_string());
    let origin = crate::server::resident::Origin::TaskEnd;
    tokio::spawn(tell_in_place(state.clone(), told, (place, started_in), (None, origin)));
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
    tell_from(state, session, agent, notice, ("system".into(), Origin::TaskEnd))
}

pub(crate) fn tell_from(
    state: std::sync::Arc<super::ServerState>,
    session: String,
    agent: String,
    notice: String,
    (sender, origin): (String, Origin),
) -> impl std::future::Future<Output = ()> + Send + 'static {
    let (_, place) = super::chat::helpers::take_turn_place(state.manager.session_turn(&session));
    tell_in_place(state, (session, agent, notice, sender), (place, None), (None, origin))
}

/// [`tell`], its notice kept under `mark` (the row's `client_id`); `kept`
/// hears once the notice is in the transcript, and is dropped if it never is.
/// `stands_in`: in a day's session, the place (a page's key) the turn
/// stands in; else the session's own.
pub(crate) fn tell_marked(
    state: std::sync::Arc<super::ServerState>,
    (session, stands_in): (String, Option<String>),
    agent: String,
    notice: String,
    mark: (String, tokio::sync::oneshot::Sender<()>),
    origin: Origin,
) -> impl std::future::Future<Output = ()> + Send + 'static {
    let (_, place) = super::chat::helpers::take_turn_place(state.manager.session_turn(&session));
    tell_in_place(
        state,
        (session, agent, notice, "system".into()),
        (place, stands_in),
        (Some(mark), origin),
    )
}

/// `started_in`: the place of the turn that started the task being told
/// of — in a day's session its end is told there; else the session's own.
async fn tell_in_place(
    state: std::sync::Arc<super::ServerState>,
    (session, agent, notice, sender): (String, String, String, String),
    (place, started_in): (super::chat::helpers::TurnSlot, Option<String>),
    (mark, told): (Option<(String, tokio::sync::oneshot::Sender<()>)>, Origin),
) {
    let Some(meta) = state
        .manager
        .global_sessions
        .get_session_meta(&session)
        .ok()
        .flatten()
    else {
        return; // the session was deleted meanwhile
    };
    if meta.is_old_claude() {
        return;
    }
    let root = std::path::PathBuf::from(meta.project.clone().unwrap_or_else(|| meta.cwd.clone()));
    let engine = match state
        .manager
        .get_or_create_session_agent(&session, &root, &agent)
        .await
    {
        Ok(engine) => engine,
        Err(error) => {
            tracing::warn!("[tasks] could not wake {agent} in {session}: {error}");
            return;
        }
    };
    // A restart's own continue turn (`mark`) is never held back; any other
    // notice still waiting for its place when a restart freezes is kept in
    // its plan instead.
    let (_turn, gate) = match &mark {
        Some(_) => (place.await, None),
        None => match place_or_plan(&state, place, (&session, &agent, &notice, &sender), told).await {
            Some((turn, gate)) => (turn, Some(gate)),
            None => return,
        },
    };
    // Where this turn comes from is recorded the moment it holds the lock: a
    // person's message that comes now ends it, and a restart's plan reads it.
    let start = super::chat::TurnStart::told(&agent, told);
    let origin = start.origin();
    let running = crate::server::resident::RunningOrigin::at_lock(&session, origin);
    // A person's message was waiting for the session: this notice's turn does
    // not run. (A restart's own continue turn, `mark`, keeps its retry
    // contract and runs.)
    let ended = running.as_ref().is_some_and(crate::server::resident::RunningOrigin::was_ended);
    if ended && mark.is_none() {
        return;
    }
    // A plan proposed while this waited for its turn holds it too.
    let Some(notice) = state
        .manager
        .hold_behind_plan(&session, &agent, notice)
        .await
    else {
        return;
    };
    // Its place is kept before its notice is: the notice — a task's output
    // tail — is a row of that place.
    let told_at = super::chat::turn_place::told_in(&state.manager, &session, started_in.as_deref());
    let _place = super::chat::turn_place::keep_in_day(&state.manager, &session, &told_at);
    match mark {
        Some((mark, kept)) => {
            let persisted = super::chat::helpers::persist_and_emit_with_images(
                &state.manager.global_sessions,
                &state.events_tx,
                &agent,
                (&sender, &agent, &notice),
                Some(&session),
                false,
                Some(&mark),
                (None, None),
                &[],
            )
            .await;
            // Not kept: no turn either, so a retry never runs it twice.
            if !persisted {
                return;
            }
            let _ = kept.send(());
        }
        None => {
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
        }
    }
    drop(gate);
    // What kind of turn this is for the one it is told to: her turns have an
    // origin, every other agent's is answered as ever.
    // Her words are in the person's language, as for every turn of hers.
    let language = crate::server::resident::language_of_turn(&state, &session, origin, None).await;
    let mut engine = engine.lock().await;
    engine.set_parent_agent(None);
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
        policy: super::chat::policy::turn_policy(super::chat::policy::TurnBy::Engine),
        sender: Some(sender),
        // Told to her, the notice is an event: what she chooses to say is
        // published by the finalizer, nothing of her reply is kept as prose.
        start,
        place: told_at.clone(),
        turn: crate::engine::turn_place::TurnSettings { language, ..Default::default() },
    };
    super::chat::run_session_turn(&ctx, &mut engine, &state.manager, None).await;
    let ended = running.as_ref().is_some_and(crate::server::resident::RunningOrigin::was_ended);
    if origin.held() && !ended {
        let reply = engine.last_assistant_text.clone().map(|t| crate::server::resident::Turned::of(t, origin));
        drop(engine);
        crate::server::resident::say_event(&state, reply.as_ref(), (&session, &told_at)).await;
    }
}

/// `place` in the session's turn line, with the admission gate to hold
/// until the notice is kept; `None` when a restart kept it in its plan
/// meanwhile (or could not, which is logged).
async fn place_or_plan(
    state: &std::sync::Arc<super::ServerState>,
    place: super::chat::helpers::TurnSlot,
    (session, agent, notice, sender): (&str, &str, &str, &str),
    origin: Origin,
) -> Option<(tokio::sync::OwnedMutexGuard<()>, super::admission::Gate)> {
    let incoming = super::admission::Incoming {
        session,
        agent,
        text: notice,
        images: &[],
        sender: Some(sender),
        place: None,
    };
    let again = || {
        resend_notice(
            state.clone(),
            session.into(),
            agent.into(),
            notice.into(),
            (sender.into(), origin),
        )
    };
    match super::admission::admit_in_line(&state.admission, place, incoming, again).await {
        Ok(held) => held,
        Err(error) => {
            tracing::error!("[tasks] {agent} in {session} not told: {error}");
            None
        }
    }
}

/// A notice kept in a restart's plan, told again as it came if this engine
/// outlives the restart.
fn resend_notice(
    state: std::sync::Arc<super::ServerState>,
    session: String,
    agent: String,
    notice: String,
    (sender, origin): (String, Origin),
) -> super::admission::Resend {
    Box::new(move || {
        Box::pin(async move {
            tokio::spawn(tell_from(state, session, agent, notice, (sender, origin)));
        })
    })
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

    // What a test does to a listed task: the engine's own paths end tasks
    // through `upsert`; these are not production code.
    impl Tasks {
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
    }

    #[test]
    fn stale_subagent_task_is_interrupted() {
        let tasks = Tasks::default();
        let mut stale = BackgroundTask::running("stale", "subagent", "review");
        stale.subagent_id = Some("ended".into());
        let mut live = BackgroundTask::running("live", "subagent", "review");
        live.subagent_id = Some("running".into());
        tasks.upsert("s", stale);
        tasks.upsert("s", live);
        tasks.upsert(
            "s",
            BackgroundTask::running("starting", "subagent", "starting"),
        );
        tasks.upsert("s", BackgroundTask::running("shell", "shell", "work"));
        assert_eq!(
            tasks.interrupt_stale_subagents(|_, t| t.subagent_id.as_deref() == Some("running")),
            vec!["s"]
        );
        assert_eq!(tasks.get("s", "stale").unwrap().status, "interrupted");
        assert!(tasks.get("s", "stale").unwrap().ended_at.is_some());
        assert!(tasks.get("s", "live").unwrap().is_running());
        assert!(tasks.get("s", "starting").unwrap().is_running());
        assert!(tasks.get("s", "shell").unwrap().is_running());
        assert!(tasks
            .interrupt_stale_subagents(|_, t| t.subagent_id.as_deref() == Some("running"))
            .is_empty());
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
        assert_eq!(
            tasks.get("s", "a").unwrap().summary.as_deref(),
            Some("built")
        );
        assert!(tasks.get("s", "a").unwrap().ended_at.is_some());
        assert!(
            !tasks.upsert("s", tasks.get("s", "b").unwrap()),
            "an unchanged task is no change"
        );
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
mod missed_end_tests {
    use super::*;

    /// The bus dropped a task's end: the engine's own list still has it, and
    /// that is where the running row learns it ended.
    #[test]
    fn a_running_row_whose_work_has_ended_is_found() {
        let tasks = Tasks::default();
        tasks.upsert("s", BackgroundTask::running("done", "shell", "make"));
        tasks.upsert("s", BackgroundTask::running("busy", "shell", "serve"));
        tasks.upsert("t", BackgroundTask::running("claude", "shell", "x"));
        let engine = |session: &str, id: &str| {
            let mut task = BackgroundTask::running(id, "shell", "x");
            match (session, id) {
                ("s", "done") => task.end("completed"),
                ("s", "busy") => {}
                _ => return None, // not the engine's (another runtime's)
            }
            Some(task)
        };
        let found = tasks.ended_unheard(engine);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "s");
        assert_eq!(found[0].1.id, "done");
        assert_eq!(found[0].1.status, "completed");
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

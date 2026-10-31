//! A native session's background tasks (doc/background-tasks-spec.md): shells
//! started with `Bash run_in_background` and subagents with `Task
//! run_in_background`. They run beside the chat. Every change goes out as
//! `AgentEvent::BackgroundTask`; the server lists it and, when a task ends,
//! gives the agent that started it a turn saying so.
use crate::engine::agent::{AgentEvent, AgentManager};
use crate::engine::events::BackgroundTask;
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

/// Lines of a shell's output the ending turn quotes.
const NOTICE_TAIL: usize = 20;
/// How often a running shell's latest output line is refreshed in the list.
const ACTIVITY_EVERY: Duration = Duration::from_secs(2);

struct Entry {
    session: String,
    owner: Option<String>,
    child_owner: bool,
    task: BackgroundTask,
    result: Option<String>,
    /// A running shell's stop.
    stop: Option<oneshot::Sender<()>>,
    /// A background subagent's run, which a stop cancels.
    run_id: Option<String>,
    /// Its session is ending (Clear, delete): its end is told to no one.
    silent: bool,
}

/// What stopping a task takes.
pub enum Stop {
    Sent,
    CancelRun(String),
}

#[derive(Default)]
pub struct Background {
    entries: Mutex<HashMap<String, Entry>>,
}

fn locked<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Background {
    /// A task of this session (never another session's).
    #[cfg(test)]
    pub fn get(&self, session: &str, id: &str) -> Option<BackgroundTask> {
        locked(&self.entries)
            .get(id)
            .filter(|e| e.session == session)
            .map(|e| e.task.clone())
    }

    /// Agent tools see only work they started, even within the same session.
    pub fn list_owned(&self, session: &str, owner: Option<&str>) -> Vec<BackgroundTask> {
        let mut tasks: Vec<_> = locked(&self.entries).values()
            .filter(|e| e.session == session && e.owner.as_deref() == owner)
            .map(|e| e.task.clone()).collect();
        tasks.sort_by(|a, b| a.id.cmp(&b.id));
        tasks
    }

    pub fn get_owned(&self, session: &str, owner: Option<&str>, id: &str) -> Option<(BackgroundTask, Option<String>)> {
        locked(&self.entries).get(id)
            .filter(|e| e.session == session && e.owner.as_deref() == owner)
            .map(|e| (e.task.clone(), e.result.clone()))
    }

    pub fn stop_owned(&self, session: &str, owner: Option<&str>, id: &str) -> Result<Stop> {
        let mut entries = locked(&self.entries);
        let entry = entries.get_mut(id)
            .filter(|e| e.session == session && e.owner.as_deref() == owner)
            .context("No such task in your scope")?;
        stop_entry(entry, id)
    }

    pub fn stop(&self, session: &str, id: &str) -> Result<Stop> {
        let mut entries = locked(&self.entries);
        let entry = entries
            .get_mut(id)
            .filter(|e| e.session == session)
            .context("No such background task in this session")?;
        stop_entry(entry, id)
    }

    /// Stop every running task of `session`, silently: their ends are told
    /// to no agent. Returns what each stop takes and the stopped tasks' ids.
    fn stop_session(&self, session: &str) -> (Vec<Stop>, Vec<String>) {
        let mut entries = locked(&self.entries);
        let mut stops = Vec::new();
        let mut ids = Vec::new();
        for entry in entries.values_mut().filter(|e| e.session == session && e.task.is_running()) {
            entry.silent = true;
            ids.push(entry.task.id.clone());
            if let Some(stop) = entry.stop.take() {
                let _ = stop.send(());
                stops.push(Stop::Sent);
            } else if let Some(run) = entry.run_id.clone() {
                stops.push(Stop::CancelRun(run));
            }
        }
        (stops, ids)
    }

    fn running(&self, ids: &[String]) -> bool {
        let entries = locked(&self.entries);
        ids.iter().any(|id| entries.get(id).is_some_and(|e| e.task.is_running()))
    }

    /// Route child-owned notices only to that live run, never to a session turn.
    fn notice(&self, manager: &AgentManager, id: &str, notice: String) -> Option<String> {
        let entries = locked(&self.entries);
        let entry = entries.get(id)?;
        if entry.silent { return None; }
        if entry.child_owner {
            if let Some(owner) = entry.owner.as_deref() {
                manager.queue_run_notice(owner, notice);
            }
            return None;
        }
        Some(notice)
    }

    fn put(&self, session: &str, owner: Option<&str>, child_owner: bool, task: BackgroundTask, stop: Option<oneshot::Sender<()>>, run_id: Option<String>) {
        locked(&self.entries).insert(
            task.id.clone(),
            Entry { session: session.into(), owner: owner.map(str::to_owned), child_owner, task, result: None, stop, run_id, silent: false },
        );
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut BackgroundTask)) -> Option<BackgroundTask> {
        let mut entries = locked(&self.entries);
        let entry = entries.get_mut(id)?;
        f(&mut entry.task);
        Some(entry.task.clone())
    }
}

fn stop_entry(entry: &mut Entry, id: &str) -> Result<Stop> {
        if !entry.task.is_running() {
            bail!("Task {id} already ended ({})", entry.task.status);
        }
        if let Some(stop) = entry.stop.take() {
            let _ = stop.send(());
            return Ok(Stop::Sent);
        }
        entry
            .run_id
            .clone()
            .map(Stop::CancelRun)
            .context("Task cannot be stopped")
}

async fn emit(manager: &AgentManager, session: &str, agent: &str, task: BackgroundTask, notice: Option<String>) {
    manager
        .send_event(
            AgentEvent::BackgroundTask { agent_id: agent.into(), task, notice },
            Some(session.into()),
        )
        .await;
}

fn short_id(prefix: &str) -> String {
    format!("{prefix}-{}", &uuid::Uuid::new_v4().simple().to_string()[..8])
}

/// The last `lines` lines of a task's output file (its last 64 KB at most).
pub fn tail(path: &Path, lines: usize) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(path) else {
        return String::new();
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = file.seek(SeekFrom::Start(len.saturating_sub(64 * 1024)));
    let mut bytes = Vec::new();
    let _ = file.read_to_end(&mut bytes);
    let text = String::from_utf8_lossy(&bytes);
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// Start `cmd` as a background shell of this session, in `cwd`. Returns at
/// once; the shell's output goes to a file in the session's directory.
pub async fn spawn_shell(
    manager: &Arc<AgentManager>,
    session: &str,
    agent: &str,
    owner: Option<&str>,
    child_owner: bool,
    cmd: &str,
    cwd: &Path,
) -> Result<BackgroundTask> {
    let id = short_id("sh");
    let dir = manager.global_sessions.session_dir(session).join("tasks");
    std::fs::create_dir_all(&dir)?;
    let output = dir.join(format!("{id}.log"));
    let file = std::fs::File::create(&output)?;
    let mut command = tokio::process::Command::new("sh");
    command
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        // The same PATH a foreground Bash gets (cargo, brew, the user's own
        // bin folders); the engine's own is a launchd one without them.
        .env("PATH", crate::util::shell_path())
        .env(crate::paths::SESSION_ENV, session)
        .stdin(std::process::Stdio::null())
        .stdout(file.try_clone()?)
        .stderr(file);
    // Its own process group, so a stop ends whatever it started.
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().context("Could not start the background shell")?;
    let pid = child.id();
    if let Some(pid) = pid {
        let cmd = cmd.to_owned();
        ledger_edit(|rows| rows.push((pid, cmd)));
    }
    let mut task = BackgroundTask::running(&id, "shell", cmd);
    task.output_file = Some(output.to_string_lossy().into_owned());
    let (stop_tx, mut stop_rx) = oneshot::channel();
    manager.background.put(session, owner, child_owner, task.clone(), Some(stop_tx), None);
    emit(manager, session, agent, task.clone(), None).await;

    let (manager, session, agent, cmd) = (manager.clone(), session.to_owned(), agent.to_owned(), cmd.to_owned());
    tokio::spawn(async move {
        let mut stopped = false;
        let mut seen = String::new();
        let mut tick = tokio::time::interval(ACTIVITY_EVERY);
        let status = loop {
            tokio::select! {
                status = child.wait() => break status.ok(),
                _ = &mut stop_rx, if !stopped => {
                    stopped = true;
                    kill_group(pid, libc::SIGTERM);
                    let ended = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
                    if ended.is_err() {
                        kill_group(pid, libc::SIGKILL);
                    }
                    break child.wait().await.ok();
                }
                _ = tick.tick() => {
                    let last = tail(&output, 1);
                    if !last.is_empty() && last != seen {
                        seen = last.clone();
                        if let Some(task) = manager.background.update(&id, |t| t.activity = Some(last)) {
                            emit(&manager, &session, &agent, task, None).await;
                        }
                    }
                }
            }
        };
        if let Some(pid) = pid {
            ledger_edit(|rows| rows.retain(|(p, _)| *p != pid));
        }
        let code = status.and_then(|s| s.code());
        let (state, summary) = match (stopped, code) {
            (true, _) => ("stopped", "stopped".to_string()),
            (false, Some(0)) => ("completed", "exit code 0".to_string()),
            (false, Some(code)) => ("failed", format!("exit code {code}")),
            (false, None) => ("failed", "ended by a signal".to_string()),
        };
        let quoted = tail(&output, NOTICE_TAIL);
        let notice = format!(
            "Background task {id} ended: {state} ({summary}).\nCommand: {cmd}\nLast output:\n{}\nFull output: {}",
            if quoted.is_empty() { "(none)" } else { &quoted },
            output.display()
        );
        let notice = manager.background.notice(&manager, &id, notice);
        if let Some(task) = manager.background.update(&id, |t| {
            t.end(state);
            t.summary = Some(summary);
        }) {
            emit(&manager, &session, &agent, task, notice).await;
        }
    });
    Ok(task)
}

/// The session is ending (Clear, delete): stop every task it still runs and
/// wait, a few seconds at most, until they have ended. Their ends are told to
/// no agent.
pub async fn end_session(manager: &AgentManager, session: &str) {
    let (stops, ids) = manager.background.stop_session(session);
    for stop in stops {
        if let Stop::CancelRun(run) = stop {
            let _ = manager.cancel_run_tree(&run).await;
        }
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while manager.background.running(&ids) && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(unix)]
fn kill_group(pid: Option<u32>, signal: i32) {
    if let Some(pid) = pid {
        unsafe {
            libc::killpg(pid as i32, signal);
        }
    }
}

#[cfg(not(unix))]
fn kill_group(_pid: Option<u32>, _signal: i32) {}

/// List a background subagent once its run exists. `run` resolves to its
/// outcome text; the subagent's own run id comes on `started`.
pub async fn track_subagent(
    manager: &Arc<AgentManager>,
    session: &str,
    agent: &str,
    target: &str,
    owner: Option<&str>,
    child_owner: bool,
    task_text: &str,
    started: oneshot::Receiver<String>,
    run: impl std::future::Future<Output = Result<String>> + Send + 'static,
) -> BackgroundTask {
    let id = short_id("ag");
    let title: String = task_text.chars().take(200).collect();
    let task = BackgroundTask::running(&id, "subagent", &format!("{target}: {title}"));
    let (stop_tx, mut stop_rx) = oneshot::channel();
    manager.background.put(session, owner, child_owner, task.clone(), Some(stop_tx), None);
    let (manager, session, agent, target, id2) =
        (manager.clone(), session.to_owned(), agent.to_owned(), target.to_owned(), id.clone());
    let initial = task.clone();
    tokio::spawn(async move {
        emit(&manager, &session, &agent, initial, None).await;
        // The stop channel exists before the worker starts. A stop during startup
        // is remembered and applied as soon as its run id becomes available.
        let mut started = Some(started);
        let mut run_id: Option<String> = None;
        let mut stopping = false;
        let mut stop_open = true;
        tokio::pin!(run);
        let result = loop {
            tokio::select! {
                biased;
                signal = &mut stop_rx, if stop_open => {
                    stop_open = false;
                    if signal.is_ok() {
                        stopping = true;
                        if let Some(id) = &run_id {
                            let _ = manager.cancel_run_tree(id).await;
                        }
                    }
                }
                id = async { started.as_mut().unwrap().await }, if started.is_some() => {
                    started = None;
                    if let Ok(id) = id {
                        if let Some(task) = manager.background.update(&id2, |t| t.subagent_id = Some(id.clone())) {
                            emit(&manager, &session, &agent, task, None).await;
                        }
                        if stopping {
                            let _ = manager.cancel_run_tree(&id).await;
                        }
                        run_id = Some(id);
                    }
                }
                result = &mut run => break result,
            }
        };
        let (state, text) = match &result {
            Ok(text) => ("completed", text.clone()),
            Err(error) if error.is::<crate::engine::tools::DelegationCancelled>() => ("stopped", error.to_string()),
            Err(error) => ("failed", error.to_string()),
        };
        if let Some(entry) = locked(&manager.background.entries).get_mut(&id2) {
            entry.result = Some(text.clone());
        }
        let clipped: String = text.chars().take(4000).collect();
        let notice = format!("Background subagent {target} ({id2}) ended: {state}.\n{clipped}");
        let notice = manager.background.notice(&manager, &id2, notice);
        if let Some(task) = manager.background.update(&id2, |t| {
            t.end(state);
            t.summary = Some(clipped.chars().take(200).collect());
        }) {
            emit(&manager, &session, &agent, task, notice).await;
        }
    });
    task
}

/// The shells this engine started and that still run: their process groups
/// outlive the engine, so the next engine ends them (an engine restart ends
/// background tasks).
fn ledger() -> PathBuf {
    crate::paths::linggen_home().join("runtime/background-shells.json")
}

fn ledger_edit(f: impl FnOnce(&mut Vec<(u32, String)>)) {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = locked(&LOCK);
    let path = ledger();
    let mut rows: Vec<(u32, String)> = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    f(&mut rows);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, serde_json::to_vec(&rows).unwrap_or_default());
}

/// End the shells a previous engine left running. A pid is only signalled
/// when it still runs the same command line, never a reused pid.
pub fn end_leftovers() {
    ledger_edit(|rows| {
        for (pid, cmd) in rows.drain(..) {
            let running = std::process::Command::new("ps")
                .args(["-o", "command=", "-p", &pid.to_string()])
                .output()
                .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
                .unwrap_or_default();
            if running == format!("sh -c {cmd}") {
                tracing::info!("[tasks] ending background shell {pid} a previous engine left: {cmd}");
                kill_group(Some(pid), libc::SIGKILL);
            }
        }
    });
}

/// Where a task's output is, when it has a file.
pub fn output_of(task: &BackgroundTask) -> Option<PathBuf> {
    task.output_file.as_ref().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager() -> Arc<AgentManager> {
        AgentManager::new(
            crate::config::Config::default(),
            None,
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            crate::engine::InterfaceMode::Web,
        ).0
    }

    #[tokio::test]
    async fn stop_before_subagent_start_cancels_its_run() {
        let manager = manager();
        let (started_tx, started_rx) = oneshot::channel();
        let (end_tx, end_rx) = oneshot::channel::<()>();
        let task = track_subagent(&manager, "s", "ling", "worker", None, false, "test", started_rx, async move {
            let _ = end_rx.await;
            Err(crate::engine::tools::DelegationCancelled.into())
        }).await;
        assert!(matches!(manager.background.stop("s", &task.id), Ok(Stop::Sent)));
        let run = manager.begin_delegated_run(Path::new("."), Some("s"), "worker", Some("parent".into()), None, true).await.unwrap();
        started_tx.send(run.clone()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !manager.is_run_cancelled(&run).await {
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        end_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while manager.background.get("s", &task.id).unwrap().is_running() {
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        assert_eq!(manager.background.get("s", &task.id).unwrap().status, "stopped");
        manager.finish_agent_run(&run, crate::engine::agent::AgentRunStatus::Completed, None).await.unwrap();
    }

    #[test]
    fn task_tools_are_scoped_to_session_and_creator() {
        let bg = Background::default();
        bg.put("s", None, false, BackgroundTask::running("root", "shell", "x"), None, None);
        bg.put("s", Some("child"), true, BackgroundTask::running("child-task", "shell", "x"), None, None);
        bg.put("other", None, false, BackgroundTask::running("other", "shell", "x"), None, None);
        assert_eq!(bg.list_owned("s", None).len(), 1);
        assert_eq!(bg.list_owned("s", Some("child"))[0].id, "child-task");
        assert!(bg.get_owned("s", None, "child-task").is_none());
        assert!(bg.stop_owned("s", None, "child-task").is_err());
        assert!(bg.get_owned("other", None, "root").is_none());
    }

    #[tokio::test]
    async fn child_notice_never_wakes_session_and_reaches_only_live_owner() {
        let manager = manager();
        let owner = manager.begin_delegated_run(Path::new("."), Some("s"), "child", None, None, true).await.unwrap();
        let bg = Background::default();
        bg.put("s", Some(&owner), true, BackgroundTask::running("child-task", "shell", "x"), None, None);
        assert!(bg.notice(&manager, "child-task", "finished".into()).is_none());
        assert_eq!(manager.drain_run_messages(&owner), vec!["finished"]);
        manager.finish_agent_run(&owner, crate::engine::agent::AgentRunStatus::Completed, None).await.unwrap();
        assert!(bg.notice(&manager, "child-task", "late".into()).is_none());
    }

    #[test]
    fn tail_keeps_the_last_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.log");
        std::fs::write(&path, "a\nb\nc\nd\n").unwrap();
        assert_eq!(tail(&path, 2), "c\nd");
        assert_eq!(tail(&path, 10), "a\nb\nc\nd");
        assert_eq!(tail(&dir.path().join("none"), 3), "");
    }

    #[test]
    fn a_task_belongs_to_its_session_and_stops_once() {
        let bg = Background::default();
        let (tx, mut rx) = oneshot::channel();
        bg.put("s", None, false, BackgroundTask::running("sh-1", "shell", "sleep 9"), Some(tx), None);
        assert!(bg.get("other", "sh-1").is_none());
        assert!(bg.stop("other", "sh-1").is_err());
        assert!(matches!(bg.stop("s", "sh-1"), Ok(Stop::Sent)));
        assert!(rx.try_recv().is_ok());
        bg.update("sh-1", |t| t.end("stopped"));
        assert!(bg.stop("s", "sh-1").is_err(), "an ended task cannot be stopped");
        bg.put("s", None, false, BackgroundTask::running("ag-1", "subagent", "x"), None, Some("run9".into()));
        assert!(matches!(bg.stop("s", "ag-1"), Ok(Stop::CancelRun(r)) if r == "run9"));
    }

    #[test]
    fn an_ending_session_stops_all_its_tasks_silently_and_no_others() {
        let bg = Background::default();
        let (tx, mut rx) = oneshot::channel();
        let (other_tx, mut other_rx) = oneshot::channel();
        bg.put("s", None, false, BackgroundTask::running("sh-1", "shell", "sleep 9"), Some(tx), None);
        bg.put("s", None, false, BackgroundTask::running("ag-1", "subagent", "x"), None, Some("run9".into()));
        bg.put("t", None, false, BackgroundTask::running("sh-2", "shell", "sleep 9"), Some(other_tx), None);
        let (stops, ids) = bg.stop_session("s");
        assert_eq!(stops.len(), 2);
        assert!(stops.iter().any(|s| matches!(s, Stop::CancelRun(r) if r == "run9")));
        assert!(rx.try_recv().is_ok());
        assert!(other_rx.try_recv().is_err(), "another session's task was stopped");
        assert!(bg.running(&ids));
        assert!(bg.notice(&manager(), "sh-1", "end".into()).is_none());
        assert!(bg.notice(&manager(), "ag-1", "end".into()).is_none());
        assert!(bg.notice(&manager(), "sh-2", "end".into()).is_some());
        bg.update("sh-1", |t| t.end("stopped"));
        bg.update("ag-1", |t| t.end("stopped"));
        assert!(!bg.running(&ids));
    }
}

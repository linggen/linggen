//! What the engine keeps for live Claude sessions, in memory only: each
//! session's live runner (it outlives a turn while the session has work,
//! doc/background-tasks-spec.md), control channels into it, runner processes
//! (so a wedged one can be killed), the session's "Allow for this session" rules and probe results.
//! None of it outlives the engine, the way a Claude Code session's rules die
//! with its process. A folder-trust "no" is the session's own file instead
//! (`trust.rs`): it must hold across an engine restart.
use super::Control;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as SyncMutex};
use tokio::sync::{mpsc, oneshot, Mutex};

/// A session's runner while it lives: where its turns go.
pub(crate) struct Live {
    pub turns: mpsc::UnboundedSender<super::protocol::TurnRequest>,
    /// What the runner was started on (model, folder trust, MCP servers,
    /// withheld tools); a turn on other settings needs a fresh runner.
    pub settings: Arc<SyncMutex<String>>,
    pub ended: Arc<tokio::sync::Notify>,
}

#[derive(Default)]
pub(crate) struct Host {
    pub controls: Mutex<HashMap<String, mpsc::UnboundedSender<Control>>>,
    pub live: Mutex<HashMap<String, Live>>,
    runners: SyncMutex<HashMap<String, u32>>,
    rules: SyncMutex<HashMap<String, Vec<Value>>>,
    /// Queued messages waiting for the turn, by queue item id: a Steer sends
    /// one into the running turn instead.
    steers: SyncMutex<HashMap<String, oneshot::Sender<()>>>,
    pub(super) probes: Mutex<HashMap<String, Arc<Mutex<Option<super::probe::Outcome>>>>>,
}

fn locked<T>(mutex: &SyncMutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Host {
    pub fn runner_started(&self, session: &str, pid: u32) {
        locked(&self.runners).insert(session.into(), pid);
    }

    pub fn runner_ended(&self, session: &str, pid: u32) {
        let mut runners = locked(&self.runners);
        if runners.get(session) == Some(&pid) {
            runners.remove(session);
        }
    }

    /// Kill a session's runner process group outright.
    pub fn kill_runner(&self, session: &str) -> bool {
        let Some(pid) = locked(&self.runners).remove(session) else {
            return false;
        };
        #[cfg(unix)]
        unsafe {
            libc::killpg(pid as i32, libc::SIGKILL);
        }
        true
    }

    /// The SDK permission updates approved for this session's lifetime.
    pub fn session_rules(&self, session: &str) -> Vec<Value> {
        locked(&self.rules)
            .get(session)
            .cloned()
            .unwrap_or_default()
    }

    pub fn add_session_rules(&self, session: &str, updates: &[Value]) {
        locked(&self.rules)
            .entry(session.into())
            .or_default()
            .extend(updates.iter().cloned());
    }

    /// A session's conversation ended (Clear, SDK reset, delete): what was
    /// approved for it goes with it.
    pub fn forget_session(&self, session: &str) {
        locked(&self.rules).remove(session);
    }

    /// A queued message waits here until its turn comes or the person
    /// steers it into the running one.
    pub fn await_steer(&self, queued_id: &str) -> oneshot::Receiver<()> {
        let (tx, rx) = oneshot::channel();
        locked(&self.steers).insert(queued_id.into(), tx);
        rx
    }

    pub fn forget_steer(&self, queued_id: &str) {
        locked(&self.steers).remove(queued_id);
    }

    /// False when the message is no longer waiting (already sent or dropped).
    pub fn steer(&self, queued_id: &str) -> bool {
        locked(&self.steers)
            .remove(queued_id)
            .is_some_and(|tx| tx.send(()).is_ok())
    }

}

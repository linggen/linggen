//! What the engine keeps for live Claude sessions, in memory only: control
//! channels into running turns, runner processes (so a wedged one can be
//! killed), the session's "Allow for this session" rules and probe results.
//! None of it outlives the engine, the way a Claude Code session's rules die
//! with its process.
use super::Control;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex as SyncMutex};
use tokio::sync::{mpsc, Mutex};

#[derive(Default)]
pub(crate) struct Host {
    pub controls: Mutex<HashMap<String, mpsc::UnboundedSender<Control>>>,
    runners: SyncMutex<HashMap<String, u32>>,
    rules: SyncMutex<HashMap<String, Vec<Value>>>,
    declined: SyncMutex<HashSet<String>>,
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
    /// approved or declined for it goes with it.
    pub fn forget_session(&self, session: &str) {
        locked(&self.rules).remove(session);
        locked(&self.declined).remove(session);
    }

    pub fn declined_trust(&self, session: &str) -> bool {
        locked(&self.declined).contains(session)
    }

    pub fn decline_trust(&self, session: &str) {
        locked(&self.declined).insert(session.into());
    }
}

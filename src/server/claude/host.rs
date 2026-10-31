//! What the engine keeps for live Claude sessions, in memory only: each
//! session's live runner (it outlives a turn while the session has work,
//! doc/background-tasks-spec.md), control channels into it, runner processes
//! (so a wedged one can be killed), the session's "Allow for this session" rules and probe results.
//! None of it outlives the engine, the way a Claude Code session's rules die
//! with its process. A folder-trust "no" is the session's own file instead
//! (`trust.rs`): it must hold across an engine restart.
use super::Control;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
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

/// How a queued message joins the running turn — the SDK's priorities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Delivery {
    /// The person pressed Steer: the SDK takes it up at once.
    Now,
    /// A tool round ended: the SDK folds it in before its next request,
    /// as Claude Code does with what was typed while it worked.
    Next,
}

impl Delivery {
    pub fn priority(self) -> &'static str {
        match self {
            Delivery::Now => "now",
            Delivery::Next => "next",
        }
    }
}

struct Waiting {
    id: String,
    session: String,
    deliver: oneshot::Sender<Delivery>,
}

#[derive(Default)]
pub(crate) struct Host {
    pub controls: Mutex<HashMap<String, mpsc::UnboundedSender<Control>>>,
    pub live: Mutex<HashMap<String, Live>>,
    runners: SyncMutex<HashMap<String, u32>>,
    rules: SyncMutex<HashMap<String, Vec<Value>>>,
    /// Queued messages waiting for the running turn, in the order they came:
    /// each goes in at the turn's next tool round, or at once on Steer.
    waiting: SyncMutex<Vec<Waiting>>,
    /// Sessions whose waiting messages are going in, one at a time.
    rounds: SyncMutex<HashSet<String>>,
    pub(super) probes: Mutex<HashMap<String, Arc<Mutex<Option<super::probe::Outcome>>>>>,
    /// The children each session's live runner is running, by tracking id.
    /// A saved child still marked running that is not here ended unseen.
    children: SyncMutex<HashMap<String, HashSet<String>>>,
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

    pub fn child_running(&self, session: &str, child: &str, running: bool) {
        let mut children = locked(&self.children);
        let set = children.entry(session.into()).or_default();
        if running {
            set.insert(child.into());
        } else {
            set.remove(child);
        }
    }

    pub fn running_children(&self, session: &str) -> HashSet<String> {
        locked(&self.children)
            .get(session)
            .cloned()
            .unwrap_or_default()
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

    /// A queued message waits here until the running turn takes it in or
    /// the turn ends.
    pub fn await_delivery(&self, session: &str, queued_id: &str) -> oneshot::Receiver<Delivery> {
        let (deliver, rx) = oneshot::channel();
        locked(&self.waiting).push(Waiting {
            id: queued_id.into(),
            session: session.into(),
            deliver,
        });
        rx
    }

    pub fn forget_waiting(&self, queued_id: &str) {
        locked(&self.waiting).retain(|w| w.id != queued_id);
    }

    /// Steer: one message goes in now. False when it is no longer waiting
    /// (already sent or dropped).
    pub fn steer(&self, queued_id: &str) -> bool {
        let mut waiting = locked(&self.waiting);
        let Some(at) = waiting.iter().position(|w| w.id == queued_id) else {
            return false;
        };
        waiting.remove(at).deliver.send(Delivery::Now).is_ok()
    }

    /// A main-thread tool round ended: the first message waiting for this
    /// session goes in. Each passes the round on once it is in
    /// (`delivered`), so they reach the turn in the order sent. False when
    /// nothing went (none waiting, or a round already runs).
    pub fn deliver_waiting(&self, session: &str) -> bool {
        let mut rounds = locked(&self.rounds);
        if rounds.contains(session) || !self.wake_next(session) {
            return false;
        }
        rounds.insert(session.into());
        true
    }

    /// The message a round woke is done: in the turn (or removed), so the
    /// next one goes; or the turn ended first, so the round is over and the
    /// rest keep their places for the next turn.
    pub fn delivered(&self, session: &str, went_in: bool) {
        let mut rounds = locked(&self.rounds);
        if went_in && self.wake_next(session) {
            return;
        }
        rounds.remove(session);
    }

    fn wake_next(&self, session: &str) -> bool {
        let mut waiting = locked(&self.waiting);
        while let Some(at) = waiting.iter().position(|w| w.session == session) {
            if waiting.remove(at).deliver.send(Delivery::Next).is_ok() {
                return true;
            }
        }
        false
    }
}

//! `sync.on-list`: a skill's command, run in the background each time a
//! device lists its items. The listing never waits on it, and a skill has at
//! most one run going — a listing that arrives during a run starts nothing.
//! See `doc/skill-spec.md` § Device sync.

use std::collections::HashSet;
use std::future::Future;
use std::sync::{Arc, Mutex};

use crate::server::ServerState;
use crate::util::LockExt;

/// Skills with an `on-list` run going.
static RUNNING: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// A skill's claim on its one run; dropping it frees the skill, a panic too.
struct Claim(String);

impl Claim {
    fn take(skill: &str) -> Option<Claim> {
        let mut running = RUNNING.lock_ok();
        let fresh = running
            .get_or_insert_with(HashSet::new)
            .insert(skill.to_string());
        fresh.then(|| Claim(skill.to_string()))
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        if let Some(running) = RUNNING.lock_ok().as_mut() {
            running.remove(&self.0);
        }
    }
}

/// Start `skill`'s run in the background unless one is going. Returns at
/// once: whether a run was started.
fn spawn_once<F>(skill: &str, run: F) -> bool
where
    F: Future<Output = ()> + Send + 'static,
{
    let Some(claim) = Claim::take(skill) else {
        return false;
    };
    tokio::spawn(async move {
        let _claim = claim;
        run.await;
    });
    true
}

/// The skill's `sync.on-list` command, in the background.
pub(super) fn kick(state: &Arc<ServerState>, skill: &str) {
    let state = state.clone();
    let name = skill.to_string();
    spawn_once(skill, async move {
        let Some(s) = state.skills.get_skill(&name).await else {
            return;
        };
        match crate::engine::skill::tools::run_on_list(&s).await {
            Ok(()) => tracing::info!("[skill-sync] {name} on-list done"),
            Err(e) => tracing::warn!("[skill-sync] {name} on-list: {e}"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn a_run_never_delays_the_caller_and_never_doubles() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let started = Instant::now();
        assert!(spawn_once("t-once", async move {
            let _ = rx.await;
        }));
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "returns at once"
        );
        assert!(
            !spawn_once("t-once", async {}),
            "one run per skill at a time"
        );
        assert!(spawn_once("t-other", async {}), "other skills are not held");
        tx.send(()).unwrap();
        for _ in 0..50 {
            if spawn_once("t-once", async {}) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("a finished run frees the skill");
    }

    #[tokio::test]
    async fn a_panicking_run_frees_the_skill() {
        assert!(spawn_once("t-panic", async { panic!("boom") }));
        for _ in 0..50 {
            if spawn_once("t-panic", async {}) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("a panic frees the skill");
    }
}

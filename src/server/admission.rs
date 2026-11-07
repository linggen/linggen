//! Who lets a message into a session while the engine restarts itself
//! (doc/background-tasks-spec.md § Engine restart). Every way a turn starts
//! — a person's message (native or Claude), a task's end, a turn of
//! Yinyue's — passes [`admit`] or [`admit_in_line`]. The restart snapshot
//! holds the gate while it reads the turns and queues, so no message is half
//! admitted then; once the plan is kept (`frozen`), every new message joins
//! the plan's queue instead of a turn the restart would end. If this engine
//! outlives the restart (the helper never stopped it), [`thaw`] hands back
//! the messages kept meanwhile, to send as they came.
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{watch, Mutex, OwnedMutexGuard};

/// How to send a message again, as it first came.
pub(crate) type Resend = Box<dyn FnOnce() -> BoxFuture<'static, ()> + Send>;

/// Held from admission until the message is queued or kept in the transcript.
pub(crate) type Gate = OwnedMutexGuard<Admission>;

pub(crate) struct Admission {
    /// The plan new messages go to: a restart is under way.
    frozen: Option<PathBuf>,
    /// The messages kept in the plan since, in the order they came.
    late: Vec<Resend>,
    /// Whether frozen, for messages waiting for their place in a turn line.
    freeze: watch::Sender<bool>,
}

impl Default for Admission {
    fn default() -> Self {
        Self {
            frozen: None,
            late: Vec::new(),
            freeze: watch::Sender::new(false),
        }
    }
}

/// One message asking in.
pub(crate) struct Incoming<'a> {
    pub(crate) session: &'a str,
    pub(crate) agent: &'a str,
    pub(crate) text: &'a str,
    pub(crate) images: &'a [String],
    /// Who sent it, when not the person (`system` for a task's end).
    pub(crate) sender: Option<&'a str>,
}

impl Incoming<'_> {
    /// As the plan's queue keeps it.
    fn item(&self) -> Value {
        let mut item = json!({"text": self.text, "images": self.images, "agent_id": self.agent});
        if let Some(sender) = self.sender {
            item["sender"] = json!(sender);
        }
        item
    }
}

/// Admit `msg`. `Ok(Some(gate))`: it goes on; hold the gate until it is
/// queued or kept in the transcript. `Ok(None)`: a restart is under way and
/// the message was kept in its plan (and `resend` with it, for [`thaw`]).
pub(crate) async fn admit(
    admission: &Arc<Mutex<Admission>>,
    msg: Incoming<'_>,
    resend: impl FnOnce() -> Resend,
) -> Result<Option<Gate>, String> {
    let gate = admission.clone().lock_owned().await;
    divert(gate, &msg, resend)
}

/// [`admit`] for a message that first waits for its place in a session's
/// turn line (`place`). The snapshot sees neither the place nor the message,
/// so a restart frozen while it waits keeps it in the plan at once, and one
/// frozen as it gets there keeps it too. `Ok(Some((place, gate)))`: it goes
/// on in its place.
pub(crate) async fn admit_in_line<T>(
    admission: &Arc<Mutex<Admission>>,
    place: impl std::future::Future<Output = T>,
    msg: Incoming<'_>,
    resend: impl FnOnce() -> Resend,
) -> Result<Option<(T, Gate)>, String> {
    let mut frozen = admission.lock().await.freeze.subscribe();
    tokio::pin!(place);
    loop {
        tokio::select! {
            biased;
            held = &mut place => {
                let gate = admission.clone().lock_owned().await;
                return Ok(divert(gate, &msg, resend)?.map(|gate| (held, gate)));
            }
            true = until_frozen(&mut frozen) => {
                let gate = admission.clone().lock_owned().await;
                if gate.frozen.is_some() {
                    return divert(gate, &msg, resend).map(|_| None);
                }
                // Thawed again before the gate was ours: wait on.
            }
        }
    }
}

/// Once frozen: true. False if the admission is gone (never, while the
/// engine runs), which leaves only the place to wait for.
async fn until_frozen(frozen: &mut watch::Receiver<bool>) -> bool {
    frozen.wait_for(|frozen| *frozen).await.is_ok()
}

/// The gate back while no restart is under way; else `msg` kept in the plan.
fn divert(
    mut gate: Gate,
    msg: &Incoming<'_>,
    resend: impl FnOnce() -> Resend,
) -> Result<Option<Gate>, String> {
    let Some(path) = gate.frozen.clone() else {
        return Ok(Some(gate));
    };
    keep_in_plan(&path, msg.session, msg.item())
        .map_err(|error| format!("Could not keep the message for after the restart: {error}"))?;
    gate.late.push(resend());
    Ok(None)
}

/// From now on every message joins the plan at `path`. Called with the gate
/// the snapshot holds.
pub(crate) fn freeze(gate: &mut Admission, path: PathBuf) {
    gate.frozen = Some(path);
    gate.freeze.send_replace(true);
}

/// This engine outlived its restart: messages go on again, the plan goes
/// (what the snapshot read — turns, queues, tasks — is all still here), and
/// the messages kept in it meanwhile come back, in order, to send again.
pub(crate) async fn thaw(admission: &Arc<Mutex<Admission>>) -> Vec<Resend> {
    let mut gate = admission.lock().await;
    let Some(path) = gate.frozen.take() else {
        return Vec::new();
    };
    if let Err(error) = std::fs::remove_file(&path) {
        tracing::warn!("[restart] could not remove {}: {error}", path.display());
    }
    gate.freeze.send_replace(false);
    std::mem::take(&mut gate.late)
}

/// Add one message to `session`'s queue in the plan, after the ones there:
/// a session with no entry gets a queue-only one.
fn keep_in_plan(path: &Path, session: &str, item: Value) -> std::io::Result<()> {
    let mut plan: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let sessions = plan["sessions"]
        .as_array_mut()
        .ok_or(std::io::ErrorKind::InvalidData)?;
    match sessions.iter_mut().find(|s| s["session_id"] == session) {
        Some(entry) => match entry["queued"].as_array_mut() {
            Some(queued) => queued.push(item),
            None => entry["queued"] = json!([item]),
        },
        None => sessions.push(json!({"session_id": session, "queued": [item]})),
    }
    super::restart::write_plan(path, &plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn msg<'a>(
        session: &'a str,
        agent: &'a str,
        text: &'a str,
        images: &'a [String],
    ) -> Incoming<'a> {
        Incoming {
            session,
            agent,
            text,
            images,
            sender: None,
        }
    }

    fn no_resend() -> Resend {
        Box::new(|| Box::pin(async {}))
    }

    /// A plan on disk, as a restart's snapshot kept it.
    fn plan_in(dir: &tempfile::TempDir) -> PathBuf {
        let path = dir.path().join("restart/resume.json");
        let plan = json!({"sessions": [{"session_id": "s", "agent_id": "ling", "queued": [{"text": "before", "images": [], "agent_id": "ling"}]}]});
        super::super::restart::write_plan(&path, &plan).unwrap();
        path
    }

    fn queued_texts(path: &Path, at: usize) -> Vec<String> {
        let kept: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        kept["sessions"][at]["queued"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m["text"].as_str().map(str::to_owned))
            .collect()
    }

    /// Once the plan is kept, a message admitted later joins the plan's
    /// queue — after what was queued, or as a queue-only session — and never
    /// a turn the restart would end. Until then, the snapshot waits for a
    /// message being admitted.
    #[tokio::test]
    async fn a_message_after_the_snapshot_waits_in_the_plan() {
        let dir = tempfile::tempdir().unwrap();
        let path = plan_in(&dir);
        let admission = Arc::new(Mutex::new(Admission::default()));
        let gate = admit(&admission, msg("s", "ling", "open", &[]), no_resend)
            .await
            .unwrap();
        assert!(gate.is_some(), "no restart yet: the message goes on");
        assert!(
            admission.try_lock().is_err(),
            "the snapshot waits while a message is admitted"
        );
        drop(gate);
        freeze(&mut *admission.lock().await, path.clone());
        let picture = ["p.png".to_string()];
        assert!(
            admit(&admission, msg("s", "ling", "after", &picture), no_resend)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            admit(&admission, msg("t", "yinyue", "elsewhere", &[]), no_resend)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(queued_texts(&path, 0), ["before", "after"]);
        let kept: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(kept["sessions"][0]["queued"][1]["images"], json!(["p.png"]));
        assert_eq!(
            kept["sessions"][1],
            json!({"session_id": "t", "queued": [{"text": "elsewhere", "images": [], "agent_id": "yinyue"}]})
        );
    }

    /// A task's end or a turn of Yinyue's waiting for its place when the
    /// restart freezes is kept in the plan at once — with who sent it — and
    /// never persisted or run; one whose place comes first goes on, the gate
    /// held until it is kept.
    #[tokio::test]
    async fn a_message_waiting_in_line_is_kept_in_the_plan_when_the_restart_freezes() {
        let dir = tempfile::tempdir().unwrap();
        let path = plan_in(&dir);
        let admission = Arc::new(Mutex::new(Admission::default()));
        let turn = Arc::new(Mutex::new(()));
        let held = turn.clone().lock_owned().await;
        let waiting = {
            let (admission, turn) = (admission.clone(), turn.clone());
            tokio::spawn(async move {
                let notice = Incoming {
                    sender: Some("system"),
                    ..msg("s", "ling", "task sh-1 ended", &[])
                };
                admit_in_line(&admission, turn.lock_owned(), notice, no_resend)
                    .await
                    .unwrap()
                    .is_some()
            })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiting.is_finished(), "it waits behind the running turn");
        freeze(&mut *admission.lock().await, path.clone());
        let went_on = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .unwrap()
            .unwrap();
        assert!(!went_on, "a message diverted at the freeze never runs here");
        assert_eq!(queued_texts(&path, 0), ["before", "task sh-1 ended"]);
        let kept: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(kept["sessions"][0]["queued"][1]["sender"], "system");
        drop(held);
        // No restart: the place is had and the gate held with it.
        let admission = Arc::new(Mutex::new(Admission::default()));
        let went = admit_in_line(
            &admission,
            turn.clone().lock_owned(),
            msg("s", "ling", "now", &[]),
            no_resend,
        )
        .await
        .unwrap();
        assert!(went.is_some());
        assert!(
            admission.try_lock().is_err(),
            "the gate is held until the message is kept"
        );
    }

    /// The engine outlived its restart: the plan goes, messages flow again,
    /// and each one kept meanwhile comes back once, in the order it came.
    #[tokio::test]
    async fn a_thawed_engine_sends_again_what_waited_in_the_plan() {
        let dir = tempfile::tempdir().unwrap();
        let path = plan_in(&dir);
        let admission = Arc::new(Mutex::new(Admission::default()));
        freeze(&mut *admission.lock().await, path.clone());
        let sent = Arc::new(std::sync::Mutex::new(Vec::new()));
        for text in ["one", "two"] {
            let sent = sent.clone();
            let resend = move || -> Resend {
                Box::new(move || Box::pin(async move { sent.lock().unwrap().push(text) }))
            };
            assert!(admit(&admission, msg("s", "ling", text, &[]), resend)
                .await
                .unwrap()
                .is_none());
        }
        let late = thaw(&admission).await;
        assert!(!path.exists(), "the plan goes with the thaw");
        for resend in late {
            resend().await;
        }
        assert_eq!(*sent.lock().unwrap(), ["one", "two"]);
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = {
            let calls = calls.clone();
            move || -> Resend {
                calls.fetch_add(1, Ordering::SeqCst);
                no_resend()
            }
        };
        assert!(admit(&admission, msg("s", "ling", "three", &[]), counted)
            .await
            .unwrap()
            .is_some());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "a message after the thaw goes on as usual"
        );
        assert!(
            thaw(&admission).await.is_empty(),
            "a second thaw has nothing to send"
        );
    }
}

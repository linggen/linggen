//! A person's messages sent while an agent's turn runs join that turn at its
//! next step, as Claude Code folds what is typed while it works into the
//! running turn (`doc/chat-spec.md` § Message queue). Until a step takes a
//! message in it stays in the queue, editable and removable.
//!
//! Each queued message waits here beside its own turn task, which holds what
//! showing it needs (sender, via, pictures). A step asks every message ready
//! for its turn, in order; each one takes itself off the queue, shows in the
//! chat, and hands its words back — or nothing, when the person removed it.
//! One that is not ready (a question is open, or the skill has every message
//! wait for the whole turn) joins only on Steer.
use crate::engine::InboxMessage;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::oneshot;

/// The reply a waiting message sends back to the step that asked for it.
pub(crate) type Reply = oneshot::Sender<Option<InboxMessage>>;

struct Waiting {
    id: String,
    /// The running turn it is for (`queue_key` of session and agent).
    key: String,
    ready: bool,
    call: oneshot::Sender<Reply>,
}

#[derive(Default)]
pub(crate) struct Fold {
    waiting: Mutex<Vec<Waiting>>,
}

/// How long a step waits for one message to show itself before going on.
const HANDOVER: Duration = Duration::from_secs(10);

impl Fold {
    fn waiting(&self) -> std::sync::MutexGuard<'_, Vec<Waiting>> {
        self.waiting
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Wait to be taken into the running turn for `key`. `ready` is false
    /// when the message should wait for the whole turn unless steered.
    pub fn wait(&self, key: &str, id: &str, ready: bool) -> oneshot::Receiver<Reply> {
        let (call, rx) = oneshot::channel();
        self.waiting().push(Waiting {
            id: id.into(),
            key: key.into(),
            ready,
            call,
        });
        rx
    }

    pub fn forget(&self, id: &str) {
        self.waiting().retain(|w| w.id != id);
    }

    /// Steer: a message that would wait for the whole turn joins it at the
    /// next step. False when it is no longer waiting here.
    pub fn steer(&self, id: &str) -> bool {
        let mut waiting = self.waiting();
        let Some(entry) = waiting.iter_mut().find(|w| w.id == id) else {
            return false;
        };
        entry.ready = true;
        true
    }

    /// A step of the turn for `key`: every ready message, in the order sent.
    pub async fn take(&self, key: &str) -> Vec<InboxMessage> {
        let calls: Vec<Waiting> = {
            let mut waiting = self.waiting();
            let (due, keep) = std::mem::take(&mut *waiting)
                .into_iter()
                .partition(|w| w.key == key && w.ready);
            *waiting = keep;
            due
        };
        let mut taken = Vec::new();
        for waiting in calls {
            let (reply, answer) = oneshot::channel();
            if waiting.call.send(reply).is_err() {
                continue;
            }
            if let Ok(Ok(Some(message))) = tokio::time::timeout(HANDOVER, answer).await {
                taken.push(message);
            }
        }
        taken
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answer a waiting message's call the way its turn task does.
    fn answer(rx: oneshot::Receiver<Reply>, text: Option<&'static str>) {
        tokio::spawn(async move {
            if let Ok(reply) = rx.await {
                let _ = reply.send(text.map(|t| InboxMessage {
                    text: t.into(),
                    images: Vec::new(),
                }));
            }
        });
    }

    #[tokio::test]
    async fn a_step_takes_ready_messages_for_its_turn_in_order() {
        let fold = Fold::default();
        answer(fold.wait("s/ling", "1", true), Some("first"));
        answer(fold.wait("s/yinyue", "2", true), Some("for another member"));
        answer(fold.wait("s/ling", "3", false), Some("waits for the turn"));
        answer(fold.wait("s/ling", "4", true), None); // removed meanwhile
        answer(fold.wait("s/ling", "5", true), Some("second"));
        let words: Vec<String> = fold
            .take("s/ling")
            .await
            .into_iter()
            .map(|m| m.text)
            .collect();
        assert_eq!(words, ["first", "second"]);
        assert!(
            fold.take("s/ling").await.is_empty(),
            "a message is taken once"
        );

        assert!(fold.steer("3"));
        let words: Vec<String> = fold
            .take("s/ling")
            .await
            .into_iter()
            .map(|m| m.text)
            .collect();
        assert_eq!(words, ["waits for the turn"]);
        assert!(!fold.steer("3"), "a taken message no longer waits");
    }
}

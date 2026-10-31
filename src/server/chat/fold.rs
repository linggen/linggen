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
/// The message carries its own ack: only a step that takes it tells it so.
pub(crate) type Reply = oneshot::Sender<Option<InboxMessage>>;

struct Waiting {
    id: String,
    /// The running turn it is for (`queue_key` of session and agent).
    key: String,
    ready: bool,
    call: oneshot::Sender<Reply>,
}

/// What a Steer did to a native queued message.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Steered {
    /// It waited for the whole turn; now it joins at the next step.
    Joins,
    /// It joins at the next step already: Steer changes nothing.
    AlreadyJoins,
    /// It no longer waits here.
    Gone,
}

#[derive(Default)]
pub(crate) struct Fold {
    waiting: Mutex<Vec<Waiting>>,
}

/// How long a step waits for one message to show itself before going on.
#[cfg(not(test))]
const HANDOVER: Duration = Duration::from_secs(10);
#[cfg(test)]
const HANDOVER: Duration = Duration::from_millis(200);

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
    /// next step.
    pub fn steer(&self, id: &str) -> Steered {
        let mut waiting = self.waiting();
        let Some(entry) = waiting.iter_mut().find(|w| w.id == id) else {
            return Steered::Gone;
        };
        if entry.ready {
            return Steered::AlreadyJoins;
        }
        entry.ready = true;
        Steered::Joins
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
            match tokio::time::timeout(HANDOVER, answer).await {
                Ok(Ok(Some(message))) => taken.push(message),
                // Removed meanwhile, or its task is gone.
                Ok(_) => {}
                // Too slow: it runs as its own turn; words that arrive now
                // drop with their ack untold. The rest are let go too, to
                // run after it as their own turns, so none overtakes it.
                Err(_) => break,
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
                let _ = reply.send(text.map(|t| words(t).0));
            }
        });
    }

    fn words(text: &str) -> (InboxMessage, oneshot::Receiver<()>) {
        let (taken, accepted) = oneshot::channel();
        let message = InboxMessage {
            text: text.into(),
            images: Vec::new(),
            taken,
        };
        (message, accepted)
    }

    /// A message whose words come after the step stopped waiting is not
    /// taken, and its task learns so: it runs as its own turn. The ones
    /// behind it are let go too, so none runs ahead of it.
    #[tokio::test]
    async fn a_late_handover_runs_as_its_own_turn_first() {
        let fold = Fold::default();
        let slow = fold.wait("s/ling", "1", true);
        let behind = fold.wait("s/ling", "2", true);
        let late = tokio::spawn(async move {
            let reply = slow.await.unwrap();
            tokio::time::sleep(HANDOVER * 2).await;
            let (message, accepted) = words("late");
            // The step's receiver is gone, or holds it unread and drops it.
            let sent = reply.send(Some(message)).is_ok();
            (sent, accepted.await.is_ok())
        });
        assert!(fold.take("s/ling").await.is_empty());
        assert!(behind.await.is_err(), "the one behind is let go to run as its own turn");
        let (_, taken) = late.await.unwrap();
        assert!(!taken, "a late message is never taken");
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

        answer(fold.wait("s/ling", "6", true), Some("third"));
        assert_eq!(fold.steer("6"), Steered::AlreadyJoins);
        assert_eq!(fold.steer("3"), Steered::Joins);
        let words: Vec<String> = fold
            .take("s/ling")
            .await
            .into_iter()
            .map(|m| m.text)
            .collect();
        assert_eq!(words, ["waits for the turn", "third"]);
        assert_eq!(fold.steer("3"), Steered::Gone, "a taken message no longer waits");
    }
}

//! Steer: the person wants a queued message in now, not at the turn's next
//! step (`doc/chat-spec.md` § Message queue). The running turn's wait points
//! poll this: a model call stops streaming and keeps what it said, a
//! foreground Bash or Task moves to the background, and the step that takes
//! the message in settles it. It means "hand off", never "kill" — that is
//! Stop.
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default, Debug)]
pub struct SteerSignal {
    fired: AtomicU64,
    settled: AtomicU64,
}

impl SteerSignal {
    /// A message was steered: whatever the turn waits on hands off.
    pub fn fire(&self) {
        self.fired.fetch_add(1, Ordering::SeqCst);
    }

    /// A steer no step has taken in yet.
    pub fn pending(&self) -> bool {
        self.fired.load(Ordering::SeqCst) > self.settled.load(Ordering::SeqCst)
    }

    /// The step about to take the queued messages in: every steer so far is
    /// answered by it. Called before the take, so a steer landing during it
    /// stays pending rather than lost.
    pub fn settle(&self) {
        self.settled
            .store(self.fired.load(Ordering::SeqCst), Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_steer_is_pending_until_a_step_settles_it() {
        let steer = SteerSignal::default();
        assert!(!steer.pending());
        steer.fire();
        assert!(steer.pending());
        steer.settle();
        assert!(!steer.pending());
        steer.fire();
        assert!(steer.pending(), "a later steer is pending again");
    }
}

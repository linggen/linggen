//! The tellings of waits: one per waiting turn — its root run, the turn tree
//! a subagent's question belongs to — while any question of it waits. A
//! question of another turn (another app's, another chat's) is its own wait.
//! Pure bookkeeping, held by `herald.rs` under one lock; nothing here wakes
//! anyone or sends anything.

use super::Told;
use std::collections::HashMap;

/// What a question that came to wait is to do.
#[derive(Debug, PartialEq)]
pub(super) enum Claim {
    /// Tell it: no telling of its wait is running or done. The token
    /// names this telling; only it may finish it.
    Tell(u64),
    /// Its wait is being told, or was: nothing new is said. `told`: the
    /// mark its card carries, when that telling was heard.
    Folded { told: Option<Told> },
}

#[derive(Default)]
struct Wait {
    /// Every question of the wait still known to wait, in the order asked.
    questions: Vec<String>,
    /// The questions a telling was tried for.
    tried: Vec<String>,
    /// The telling running now.
    telling: Option<u64>,
    /// Said: no question of this wait is told again while it lasts.
    said: bool,
    /// Heard: the mark every card of the wait carries.
    told: Option<Told>,
}

#[derive(Default)]
pub(super) struct Waits {
    by_key: HashMap<String, Wait>,
    next: u64,
}

impl Waits {
    /// Forget answered questions, and waits none of whose questions waits.
    fn prune(&mut self, pending: &[String]) {
        for wait in self.by_key.values_mut() {
            wait.questions.retain(|q| pending.contains(q));
        }
        self.by_key
            .retain(|_, w| !w.questions.is_empty() || w.telling.is_some());
    }

    fn token(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    /// Question `question_id` of wait `key` waits (`pending`: every question
    /// waiting now, this one too).
    pub(super) fn claim(&mut self, key: &str, question_id: &str, pending: &[String]) -> Claim {
        self.prune(pending);
        let token = self.next + 1;
        let wait = self.by_key.entry(key.to_string()).or_default();
        if !wait.questions.iter().any(|q| q == question_id) {
            wait.questions.push(question_id.to_string());
        }
        if wait.said || wait.telling.is_some() {
            return Claim::Folded {
                told: wait.told.clone(),
            };
        }
        wait.telling = Some(token);
        wait.tried.push(question_id.to_string());
        self.next = token;
        Claim::Tell(token)
    }

    /// Telling `token` of wait `key` was said; `told` when it was heard. The
    /// questions whose cards carry its mark — none for a stale token.
    pub(super) fn said(&mut self, key: &str, token: u64, told: Option<Told>) -> Vec<String> {
        let Some(wait) = self
            .by_key
            .get_mut(key)
            .filter(|w| w.telling == Some(token))
        else {
            return Vec::new();
        };
        wait.telling = None;
        wait.said = true;
        wait.told = told.clone();
        match told {
            Some(_) => wait.questions.clone(),
            None => Vec::new(),
        }
    }

    /// Telling `token` of wait `key` came to nothing. Another question of
    /// the wait that still waits, and was not tried, is told next: its id
    /// and the new telling's token. A stale token changes nothing.
    pub(super) fn came_to_nothing(
        &mut self,
        key: &str,
        token: u64,
        pending: &[String],
    ) -> Option<(String, u64)> {
        let wait = self
            .by_key
            .get_mut(key)
            .filter(|w| w.telling == Some(token))?;
        wait.telling = None;
        self.prune(pending);
        let next = self.token();
        let wait = self.by_key.get_mut(key)?;
        let question = wait
            .questions
            .iter()
            .find(|q| !wait.tried.contains(q))?
            .clone();
        wait.tried.push(question.clone());
        wait.telling = Some(next);
        Some((question, next))
    }

    /// The mark on `question_id`'s card: its own wait's, when heard.
    pub(super) fn told_for(&self, question_id: &str) -> Option<Told> {
        self.by_key
            .values()
            .find(|w| w.questions.iter().any(|q| q == question_id))
            .and_then(|w| w.told.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn told(at: u64) -> Told {
        Told {
            at,
            line: "They need you.".into(),
        }
    }

    /// One turn asking several times (its subagents each asking) is one
    /// wait; another turn's question in the same session is its own,
    /// told on its own, and its card carries only its own mark.
    #[test]
    fn a_wait_is_one_turn_not_one_session() {
        let mut w = Waits::default();
        let Claim::Tell(t1) = w.claim("run:app-turn", "q-app", &ids(&["q-app"])) else {
            panic!()
        };
        assert_eq!(w.said("run:app-turn", t1, Some(told(1))), ids(&["q-app"]));
        // Ling's own turn waits too, the app's question still open.
        let Claim::Tell(t2) = w.claim("run:ling-turn", "q-ling", &ids(&["q-app", "q-ling"])) else {
            panic!("another turn's question was folded into the app's wait")
        };
        assert!(
            w.told_for("q-ling").is_none(),
            "Ling's card carries the app's mark"
        );
        assert_eq!(w.said("run:ling-turn", t2, Some(told(2))), ids(&["q-ling"]));
        assert_eq!(w.told_for("q-ling").map(|t| t.at), Some(2));
        assert_eq!(w.told_for("q-app").map(|t| t.at), Some(1));
        // A subagent of Ling's turn asks: the same wait, folded, marked.
        assert_eq!(
            w.claim(
                "run:ling-turn",
                "q-sub",
                &ids(&["q-app", "q-ling", "q-sub"])
            ),
            Claim::Folded {
                told: Some(told(2))
            }
        );
        assert_eq!(w.told_for("q-sub").map(|t| t.at), Some(2));
    }

    /// A question folded into a telling that came to nothing is told next;
    /// one already answered is not.
    #[test]
    fn a_folded_question_is_told_when_the_first_telling_came_to_nothing() {
        let mut w = Waits::default();
        let Claim::Tell(t1) = w.claim("run:r", "q1", &ids(&["q1"])) else {
            panic!()
        };
        assert!(matches!(
            w.claim("run:r", "q2", &ids(&["q1", "q2"])),
            Claim::Folded { told: None }
        ));
        // q1 answered before her line was ready: q2, still waiting, is next.
        let (next, t2) = w
            .came_to_nothing("run:r", t1, &ids(&["q2"]))
            .expect("q2 is told");
        assert_eq!(next, "q2");
        // q2 too came to nothing, and no other question waits: done.
        assert!(w.came_to_nothing("run:r", t2, &ids(&["q2"])).is_none());
    }

    /// A late finish of an old telling never drops a newer wait nor opens a
    /// second telling of it.
    #[test]
    fn a_late_old_telling_never_drops_a_newer_wait() {
        let mut w = Waits::default();
        let Claim::Tell(old) = w.claim("run:r", "q1", &ids(&["q1"])) else {
            panic!()
        };
        // q1 is answered while its telling runs; q2 and q3 of the same turn
        // come to wait meanwhile: the wait is kept, they are folded.
        assert!(matches!(
            w.claim("run:r", "q2", &ids(&["q2"])),
            Claim::Folded { .. }
        ));
        assert!(matches!(
            w.claim("run:r", "q3", &ids(&["q2", "q3"])),
            Claim::Folded { .. }
        ));
        // The old telling ends with nothing: one new telling, for q2.
        let (next, new) = w
            .came_to_nothing("run:r", old, &ids(&["q2", "q3"]))
            .expect("q2 is told");
        assert_eq!(next, "q2");
        // Late again with the old token: nothing changes, nothing opens.
        assert!(
            w.came_to_nothing("run:r", old, &ids(&["q2", "q3"]))
                .is_none(),
            "a stale telling acted"
        );
        assert!(
            w.said("run:r", old, Some(told(1))).is_empty(),
            "a stale telling marked"
        );
        assert!(
            matches!(
                w.claim("run:r", "q4", &ids(&["q2", "q3", "q4"])),
                Claim::Folded { .. }
            ),
            "a second telling"
        );
        assert_eq!(
            w.said("run:r", new, Some(told(2))),
            ids(&["q2", "q3", "q4"])
        );
    }

    /// Said but not heard (no surface, her voice muted): nothing is told
    /// again, and no card is marked.
    #[test]
    fn a_telling_not_heard_marks_nothing_and_is_not_repeated() {
        let mut w = Waits::default();
        let Claim::Tell(t) = w.claim("run:r", "q1", &ids(&["q1"])) else {
            panic!()
        };
        assert!(w.said("run:r", t, None).is_empty());
        assert_eq!(
            w.claim("run:r", "q2", &ids(&["q1", "q2"])),
            Claim::Folded { told: None }
        );
        assert!(w.told_for("q1").is_none());
    }
}

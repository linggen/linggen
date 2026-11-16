//! The place of the turn that holds a session's turn lock, kept beside the
//! lock while the turn runs: a message sent meanwhile is met by it
//! ([`meets_running`]).

use super::TurnPlace;
use crate::engine::skill::QueueMode;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Each session's running turn's place, while one runs.
#[derive(Debug, Default, Clone)]
pub struct RunningPlaces {
    by_session: Arc<Mutex<HashMap<String, TurnPlace>>>,
}

/// A running turn's place, kept until this is dropped.
#[derive(Debug)]
pub struct PlaceMark {
    places: RunningPlaces,
    session: String,
}

impl Drop for PlaceMark {
    fn drop(&mut self) {
        self.places.map().remove(&self.session);
    }
}

impl RunningPlaces {
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, TurnPlace>> {
        self.by_session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Keep `place` as `session`'s running turn's, until the mark is
    /// dropped. `None` when a turn's place is kept there already: whoever
    /// marked it first holds it for the whole turn.
    pub fn mark(&self, session: &str, place: &TurnPlace) -> Option<PlaceMark> {
        let mut map = self.map();
        if map.contains_key(session) {
            return None;
        }
        map.insert(session.to_string(), place.clone());
        Some(PlaceMark {
            places: self.clone(),
            session: session.to_string(),
        })
    }

    /// The place of the turn running in `session`, when one is kept.
    pub fn of(&self, session: &str) -> Option<TurnPlace> {
        self.map().get(session).cloned()
    }
}

/// How a message sent while a turn runs in its session meets that turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Meets {
    /// The skill's rule (`skill-spec.md` § Queue): the message's and the
    /// running turn's places are one app.
    Rule(QueueMode),
    /// The running turn stands in another app: the message waits for it to
    /// end, and is never folded in or steered into it.
    AfterRun,
}

/// How a message from `incoming` meets the turn running in `running`
/// (`None`: no place is kept for it, as for a turn of the session's own
/// that no page sent). `rule` is the incoming place's skill's `queue`.
pub fn meets_running(running: Option<&TurnPlace>, incoming: &TurnPlace, rule: QueueMode) -> Meets {
    match running {
        Some(place) if !place.same_app(incoming) => Meets::AfterRun,
        _ => Meets::Rule(rule),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str) -> TurnPlace {
        TurnPlace {
            skill: Some(name.to_string()),
            ..TurnPlace::home()
        }
    }

    /// Same app: its own rule, steer or after-turn, as before places.
    #[test]
    fn in_the_same_app_the_skills_rule_holds() {
        let game = app("zz-game");
        for rule in [QueueMode::Steer, QueueMode::AfterTurn] {
            assert_eq!(meets_running(Some(&game), &game, rule), Meets::Rule(rule));
            assert_eq!(meets_running(None, &game, rule), Meets::Rule(rule));
        }
    }

    /// Different apps (or an app and home): the message waits for the
    /// running turn, whatever either declares.
    #[test]
    fn across_places_a_message_waits_for_the_running_turn() {
        let (game, ledger, home) = (app("zz-game"), app("zz-ledger"), TurnPlace::home());
        assert_eq!(meets_running(Some(&game), &ledger, QueueMode::Steer), Meets::AfterRun);
        assert_eq!(meets_running(Some(&home), &game, QueueMode::Steer), Meets::AfterRun);
        assert_eq!(meets_running(Some(&game), &home, QueueMode::Steer), Meets::AfterRun);
    }

    /// A turn's place is kept while its mark lives, by whoever marked first.
    #[test]
    fn a_running_place_is_kept_while_its_mark_lives() {
        let places = RunningPlaces::default();
        let first = places.mark("sess-1", &app("zz-game")).unwrap();
        assert!(places.mark("sess-1", &app("zz-ledger")).is_none());
        assert_eq!(places.of("sess-1").and_then(|p| p.skill), Some("zz-game".into()));
        drop(first);
        assert_eq!(places.of("sess-1"), None);
    }
}

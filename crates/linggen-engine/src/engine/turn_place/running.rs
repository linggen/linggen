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
    by_session: Arc<Mutex<HashMap<String, Running>>>,
    /// Each run's stamp, decided as it began: the app its talk is kept to,
    /// or `None` for everyone's. Newest last; a run's events carry it after
    /// its turn's mark is gone, and a subagent's run takes its parent's.
    runs: Arc<Mutex<std::collections::VecDeque<(String, Option<String>)>>>,
}

/// How many runs' stamps are remembered.
const RUNS_KEPT: usize = 1024;

/// A running turn's place, and whether its talk is kept to it.
#[derive(Debug, Clone)]
struct Running {
    place: TurnPlace,
    private: bool,
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
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, Running>> {
        self.by_session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Keep `place` as `session`'s running turn's, until the mark is
    /// dropped. `None` when a turn's place is kept there already: whoever
    /// marked it first holds it for the whole turn.
    pub fn mark(&self, session: &str, place: &TurnPlace) -> Option<PlaceMark> {
        self.mark_kept(session, place, false)
    }

    /// [`Self::mark`], with whether the turn's talk is kept to its app
    /// decided in the same step: nothing is written or sent in between.
    pub fn mark_kept(&self, session: &str, place: &TurnPlace, private: bool) -> Option<PlaceMark> {
        let mut map = self.map();
        if map.contains_key(session) {
            return None;
        }
        let running = Running {
            place: place.clone(),
            private,
        };
        map.insert(session.to_string(), running);
        Some(PlaceMark {
            places: self.clone(),
            session: session.to_string(),
        })
    }

    /// The place of the turn running in `session`, when one is kept.
    pub fn of(&self, session: &str) -> Option<TurnPlace> {
        self.map().get(session).map(|r| r.place.clone())
    }

    /// The stamp a row saved in `session` now takes: the running turn's
    /// place key, and the app its talk is kept to, if any.
    pub fn stamp_of(&self, session: &str) -> Option<(String, Option<String>)> {
        let map = self.map();
        let running = map.get(session)?;
        let key = running.place.key().to_string();
        let kept = running.private.then(|| key.clone());
        Some((key, kept))
    }

    /// The turn running in `session` keeps its talk to its app
    /// (`turn_place::sight`): what it streams goes to no surface outside it.
    pub fn keep_talk_private(&self, session: &str) {
        if let Some(running) = self.map().get_mut(session) {
            running.private = true;
        }
    }

    /// Stamp run `run` as it begins: a subagent's run (`parent`) takes its
    /// parent run's stamp; any other takes the stamp of the turn running in
    /// `session`. Returns the stamp.
    pub fn begin_run(&self, run: &str, parent: Option<&str>, session: Option<&str>) -> Option<String> {
        let kept = match parent.and_then(|p| self.run_stamp(p)) {
            Some(parents) => parents,
            None => session.and_then(|s| self.private_app(s)),
        };
        let mut runs = self.runs.lock().unwrap_or_else(|p| p.into_inner());
        runs.push_back((run.to_string(), kept.clone()));
        while runs.len() > RUNS_KEPT {
            runs.pop_front();
        }
        kept
    }

    /// Run `run`'s stamp, when it was stamped as it began: `Some(None)` is
    /// everyone's, `Some(Some(app))` kept to `app`.
    pub fn run_stamp(&self, run: &str) -> Option<Option<String>> {
        let runs = self.runs.lock().unwrap_or_else(|p| p.into_inner());
        runs.iter().rev().find(|(r, _)| r == run).map(|(_, kept)| kept.clone())
    }

    /// The app the turn running in `session` keeps its talk to, if any.
    pub fn private_app(&self, session: &str) -> Option<String> {
        let map = self.map();
        let running = map.get(session).filter(|r| r.private)?;
        Some(running.place.key().to_string())
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

    /// A running turn's talk is private only once it is said to be, and
    /// only while it runs.
    #[test]
    fn a_running_turns_talk_is_private_while_it_runs() {
        let places = RunningPlaces::default();
        let mark = places.mark("sess-1", &app("zz-ledger")).unwrap();
        assert_eq!(places.private_app("sess-1"), None);
        places.keep_talk_private("sess-1");
        assert_eq!(places.private_app("sess-1").as_deref(), Some("zz-ledger"));
        drop(mark);
        assert_eq!(places.private_app("sess-1"), None);
        places.keep_talk_private("sess-1");
        assert_eq!(places.private_app("sess-1"), None, "nothing runs");
    }

    /// A turn marked kept stamps its rows kept from its very first one.
    #[test]
    fn a_turn_marked_kept_is_kept_from_the_start() {
        let places = RunningPlaces::default();
        let _mark = places.mark_kept("sess-1", &app("zz-ledger"), true).unwrap();
        let stamp = places.stamp_of("sess-1");
        assert_eq!(stamp, Some(("zz-ledger".into(), Some("zz-ledger".into()))));
    }

    /// A run is stamped as it begins, from the turn running then; its
    /// subagents take its stamp, and the stamp outlives the turn's mark.
    #[test]
    fn a_runs_stamp_is_decided_as_it_begins_and_its_subagents_inherit_it() {
        let places = RunningPlaces::default();
        let mark = places.mark("sess-1", &app("zz-ledger")).unwrap();
        places.keep_talk_private("sess-1");
        assert_eq!(places.begin_run("run-1", None, Some("sess-1")).as_deref(), Some("zz-ledger"));
        drop(mark);
        assert_eq!(places.run_stamp("run-1"), Some(Some("zz-ledger".into())));
        // Begun after the turn ended, a subagent still takes its parent's.
        assert_eq!(places.begin_run("run-2", Some("run-1"), Some("sess-1")).as_deref(), Some("zz-ledger"));
        // A home turn's run is everyone's, whatever runs later.
        let _home = places.mark("sess-1", &TurnPlace::home()).unwrap();
        assert_eq!(places.begin_run("run-3", None, Some("sess-1")), None);
        assert_eq!(places.run_stamp("run-3"), Some(None));
        assert_eq!(places.run_stamp("run-9"), None);
    }
}

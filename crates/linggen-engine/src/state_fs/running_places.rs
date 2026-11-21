//! The place of the turn that holds a session's turn lock, kept beside the
//! lock while the turn runs.

use super::turn_place::TurnPlace;
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

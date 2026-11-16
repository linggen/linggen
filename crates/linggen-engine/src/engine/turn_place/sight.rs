//! What a turn may be given of a session's rows. An app that declares its
//! talk private (`private-talk: true`) keeps it to itself: a row written in
//! its place — words, tool calls and results, recall, a summary written in
//! its turn — is given to no turn standing in another place, and shown to
//! no surface outside it (`doc/one-ling-spec.md` § One session a day).
//!
//! A row in the session's own place is never hidden: an app's own session
//! holds only its own talk, so this changes nothing outside a session that
//! more than one app writes into (the day's session). A kept world block is
//! never hidden either: it holds world lines, which every place is given.

use super::TurnPlace;
use crate::engine::skill::SkillRegistry;
use crate::state_fs::sessions::{ChatMsg, SessionMeta};
use std::collections::HashSet;

/// Where a reader stands, and which apps keep their talk to themselves.
#[derive(Debug, Clone, Default)]
pub struct Sight {
    /// The reader's place key (`TurnPlace::key`).
    reader: String,
    /// The apps that declare `private-talk: true`, by name.
    private: HashSet<String>,
}

impl Sight {
    /// A reader standing in `place`.
    pub async fn of(place: &TurnPlace, skills: &dyn SkillRegistry) -> Self {
        Self::new(place.key(), private_apps(skills).await)
    }

    /// A reader standing at home: Yinyue outside every app, the main page,
    /// the phone.
    pub async fn at_home(skills: &dyn SkillRegistry) -> Self {
        Self::of(&TurnPlace::home(), skills).await
    }

    pub fn new(reader: &str, private: HashSet<String>) -> Self {
        Self {
            reader: reader.to_string(),
            private,
        }
    }

    /// Whether the reader may be given `row`, a row of session `meta`.
    pub fn sees(&self, row: &ChatMsg, meta: &SessionMeta) -> bool {
        if self.private.is_empty() || crate::engine::prompt::world::is_world_row(row) {
            return true;
        }
        let key = row.place_in(meta);
        key == self.reader
            || !self.private.contains(&key)
            || key == TurnPlace::of_session(Some(meta)).key()
    }

    /// The rows of `meta`'s session the reader may be given, in order.
    pub fn visible(&self, rows: Vec<ChatMsg>, meta: &SessionMeta) -> Vec<ChatMsg> {
        rows.into_iter().filter(|r| self.sees(r, meta)).collect()
    }

    /// Whether talk written in `place` is hidden from readers in other
    /// places of `meta`'s session.
    pub fn hides_from_others(&self, place: &TurnPlace, meta: &SessionMeta) -> bool {
        self.private.contains(place.key()) && place.key() != TurnPlace::of_session(Some(meta)).key()
    }
}

/// The installed apps that declare their talk private.
pub async fn private_apps(skills: &dyn SkillRegistry) -> HashSet<String> {
    skills
        .list_skills()
        .await
        .into_iter()
        .filter(|s| s.private_talk)
        .map(|s| s.name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::turn_place::HOME_KEY;

    fn row(place: Option<&str>, from: &str) -> ChatMsg {
        ChatMsg {
            agent_id: "ling".into(),
            from_id: from.into(),
            to_id: "user".into(),
            content: "words".into(),
            timestamp: 1,
            is_observation: false,
            client_id: None,
            failed: false,
            via: None,
            via_note: None,
            images: Vec::new(),
            run_id: None,
            place: place.map(str::to_string),
        }
    }

    fn private() -> HashSet<String> {
        HashSet::from(["ledger".to_string()])
    }

    fn day() -> SessionMeta {
        SessionMeta {
            id: "sess-day-2026-11-16".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_private_apps_rows_are_seen_only_there() {
        let meta = day();
        let ledger = row(Some("ledger"), "user");
        assert!(!Sight::new(HOME_KEY, private()).sees(&ledger, &meta));
        assert!(!Sight::new("dice", private()).sees(&ledger, &meta));
        assert!(Sight::new("ledger", private()).sees(&ledger, &meta));
        // Another app's rows, and home's, are everyone's.
        for other in [Some("dice"), Some(HOME_KEY), None] {
            assert!(Sight::new("ledger", private()).sees(&row(other, "user"), &meta));
        }
    }

    #[test]
    fn in_the_apps_own_session_nothing_is_hidden() {
        let own = SessionMeta {
            id: "sess-1-ledger".into(),
            skill: Some("ledger".into()),
            ..Default::default()
        };
        let home = Sight::new(HOME_KEY, private());
        assert!(home.sees(&row(None, "user"), &own));
        assert!(home.sees(&row(Some("ledger"), "user"), &own));
    }

    #[test]
    fn a_kept_world_block_is_never_hidden() {
        let world = row(Some("ledger"), crate::engine::prompt::world::WORLD_SENDER);
        assert!(Sight::new(HOME_KEY, private()).sees(&world, &day()));
    }

    #[test]
    fn with_no_private_app_every_row_is_seen() {
        let rows = vec![row(Some("ledger"), "user"), row(None, "ling")];
        let seen = Sight::new(HOME_KEY, HashSet::new()).visible(rows.clone(), &day());
        assert_eq!(seen, rows);
    }
}

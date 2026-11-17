//! What a turn may be given of a session's rows. An app that declares its
//! talk private (`private-talk: true`) keeps it to itself: a row written in
//! its place — words, tool calls and results, recall, a summary written in
//! its turn — is given to no turn standing in another place, and shown to
//! no surface outside it (`doc/one-ling-spec.md` § One session a day).
//!
//! Whether a row is kept is decided once, as it is written ([`kept_to`]), and
//! stamped on the row (`ChatMsg::kept_to`). A reader filters on the stamp
//! alone: nothing about privacy is looked up as rows are read, so a row
//! stays where it was kept whatever is installed later.
//!
//! A row in the session's own place is never kept: an app's own session
//! holds only its own talk, so this changes nothing outside a session that
//! more than one app writes into (the day's session). A kept world block is
//! never hidden either: it holds world lines, which every place is given.

use super::TurnPlace;
use crate::engine::compaction::COMPACTION_SENDER;
use crate::engine::skill::SkillRegistry;
use crate::state_fs::sessions::{ChatMsg, SessionMeta};

/// Where a reader stands, and the app its own writes are kept to.
#[derive(Debug, Clone, Default)]
pub struct Sight {
    /// The reader's place key (`TurnPlace::key`).
    reader: String,
    /// The app what this reader writes is kept to, when it stands in a
    /// private app's place: its thread starts from that app's own summary.
    keeps: Option<String>,
}

impl Sight {
    /// A turn standing in `place` of session `meta`: what it writes is kept
    /// as [`kept_to`] decides, now.
    pub async fn of(place: &TurnPlace, meta: &SessionMeta, skills: &dyn SkillRegistry) -> Self {
        Self {
            reader: place.key().to_string(),
            keeps: kept_to(place, meta, skills).await,
        }
    }

    /// A page of the place `key` (an app's, or home's): it is shown what that
    /// place may see.
    pub fn page(key: &str) -> Self {
        Self {
            reader: key.to_string(),
            keeps: None,
        }
    }

    /// The app what this reader writes is kept to, if any.
    pub fn keeps(&self) -> Option<&str> {
        self.keeps.as_deref()
    }

    /// Whether the reader may be given `row`: by the row's stamp alone.
    pub fn sees(&self, row: &ChatMsg) -> bool {
        if crate::engine::prompt::world::is_world_row(row) {
            return true;
        }
        if !self.starts_from(row) {
            return false;
        }
        row.kept_to.as_deref().is_none_or(|app| app == self.reader)
    }

    /// A reader in a private app reads its thread from that app's own
    /// summary: a summary written elsewhere does not stand for its talk.
    fn starts_from(&self, row: &ChatMsg) -> bool {
        let Some(own) = self.keeps.as_deref() else {
            return true;
        };
        row.from_id != COMPACTION_SENDER || row.is_observation || row.kept_to.as_deref() == Some(own)
    }

    /// The rows the reader may be given, in order.
    pub fn visible(&self, rows: Vec<ChatMsg>) -> Vec<ChatMsg> {
        rows.into_iter().filter(|r| self.sees(r)).collect()
    }
}

/// The app the talk of a turn standing in `place` of session `meta` is kept
/// to: the place's app when it declares `private-talk: true` and is not the
/// session's own place. In a day's session an app that does not resolve now
/// (removed, or its SKILL.md unreadable) keeps its talk too: when it is not
/// known to be shared, it is kept. Home's is never kept.
pub async fn kept_to(place: &TurnPlace, meta: &SessionMeta, skills: &dyn SkillRegistry) -> Option<String> {
    if place.key() == TurnPlace::of_session(Some(meta)).key() {
        return None;
    }
    let name = place.skill.as_deref()?;
    let private = match skills.get_skill(name).await {
        Some(skill) => skill.private_talk,
        None => crate::state_fs::day_session::is_day_session(&meta.id),
    };
    private.then(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::turn_place::HOME_KEY;

    fn row(kept_to: Option<&str>, from: &str) -> ChatMsg {
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
            place: kept_to.map(str::to_string),
            kept_to: kept_to.map(str::to_string),
        }
    }

    fn reader_in(key: &str, keeps: Option<&str>) -> Sight {
        Sight {
            reader: key.into(),
            keeps: keeps.map(str::to_string),
        }
    }

    #[test]
    fn a_kept_row_is_seen_only_in_its_app() {
        let ledger = row(Some("ledger"), "user");
        assert!(!Sight::page(HOME_KEY).sees(&ledger));
        assert!(!Sight::page("dice").sees(&ledger));
        assert!(Sight::page("ledger").sees(&ledger));
        // An unkept row is everyone's.
        assert!(Sight::page("ledger").sees(&row(None, "user")));
        assert!(Sight::page(HOME_KEY).sees(&row(None, "user")));
    }

    /// Nothing is looked up as rows are read: a kept row stays kept with no
    /// skill installed at all.
    #[test]
    fn a_kept_row_stays_kept_whatever_is_installed_later() {
        let rows = vec![row(Some("ledger"), "user"), row(None, "ling")];
        let seen = Sight::page(HOME_KEY).visible(rows);
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].from_id, "ling");
    }

    #[test]
    fn a_kept_world_block_is_never_hidden() {
        let world = row(Some("ledger"), crate::engine::prompt::world::WORLD_SENDER);
        assert!(Sight::page(HOME_KEY).sees(&world));
    }

    /// A private app's thread starts from its own summary, never from one
    /// written for everyone; everyone else reads everyone's summaries.
    #[test]
    fn a_private_apps_thread_starts_from_its_own_summary() {
        let everyones = row(None, COMPACTION_SENDER);
        let its_own = row(Some("ledger"), COMPACTION_SENDER);
        let ledger = reader_in("ledger", Some("ledger"));
        assert!(!ledger.sees(&everyones));
        assert!(ledger.sees(&its_own));
        assert!(reader_in("dice", None).sees(&everyones));
        assert!(!reader_in("dice", None).sees(&its_own));
    }

    #[tokio::test]
    async fn talk_is_kept_by_the_skill_as_it_is_written_and_fails_closed_in_a_day() {
        let skills = crate::engine::turn_place::tests::skills_with_private("zz-diary").await;
        let day = SessionMeta {
            id: "sess-day-2026-11-16".into(),
            ..Default::default()
        };
        let place = |name: &str| TurnPlace::of_request(Some(name), Some(&day));
        assert_eq!(kept_to(&place("zz-diary"), &day, &skills).await.as_deref(), Some("zz-diary"));
        assert_eq!(kept_to(&place("zz-gone"), &day, &skills).await.as_deref(), Some("zz-gone"));
        assert_eq!(kept_to(&TurnPlace::home(), &day, &skills).await, None);
        // The session's own place is never kept; nor is an unknown app
        // outside a day's session.
        let own = SessionMeta {
            id: "sess-1-zz-diary".into(),
            skill: Some("zz-diary".into()),
            ..Default::default()
        };
        assert_eq!(kept_to(&TurnPlace::of_session(Some(&own)), &own, &skills).await, None);
        let plain = SessionMeta {
            id: "sess-1".into(),
            ..Default::default()
        };
        let gone = TurnPlace::of_request(Some("zz-gone"), Some(&plain));
        assert_eq!(kept_to(&gone, &plain, &skills).await, None);
    }
}

//! Which session policy a turn runs under: the one place that decides what
//! every turn that is not the owner's is held to — a room guest by the
//! room's config, any other person by their own settings
//! (`server::people`), the owner by nothing. Every turn asks
//! [`turn_policy`] (lock:
//! `structure_lock_tests::which_turn_gets_the_owner_policy_is_decided_in_one_place`).
//!
//! What a person may do beyond this — their permission level — is their
//! standing's (`person::Standing`), not the policy's.

use crate::engine::session_policy::{ConsumerCeiling, SessionPolicy};
use crate::server::people::People;

/// Who stands behind a turn, as far as its policy goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TurnBy<'a> {
    /// A person's request, as the seam sent it: its `user_type` and
    /// `user_id`, with the apps installed (a person's apps are those of
    /// them they may use). The owner is under no ceiling; a room's consumer
    /// runs under the room's; any other person under their own settings.
    Person {
        user_type: &'a str,
        user_id: Option<&'a str>,
        installed: &'a [String],
    },
    /// What the engine starts itself: a task's end told to an agent, an
    /// `agent_chat` delivered, a light reply, Yinyue's own turn, an approved
    /// plan run. The engine acts for the owner (`person::owner`).
    Engine,
    /// The prompt export: what the owner's turn at the page's place sends.
    Export,
}

/// The policy a turn of `by` runs under.
pub(crate) fn turn_policy(by: TurnBy<'_>) -> SessionPolicy {
    match by {
        TurnBy::Person { user_type, user_id, installed } => {
            person_policy(user_type, user_id, installed, &crate::server::people::current())
        }
        TurnBy::Engine | TurnBy::Export => SessionPolicy::owner(),
    }
}

/// [`turn_policy`] for a person's request, with the household's `people`.
fn person_policy(user_type: &str, user_id: Option<&str>, installed: &[String], people: &People) -> SessionPolicy {
    match user_type {
        "owner" => SessionPolicy::owner(),
        "consumer" => SessionPolicy::consumer(room_ceiling()),
        _ => {
            let person = crate::server::person::asking(user_id);
            if person.is_owner() {
                return SessionPolicy::owner();
            }
            let ceiling = people.ceiling_of(&person, installed);
            SessionPolicy::person(ceiling.skills, ceiling.models)
        }
    }
}

/// What the owner's room config lets a consumer use, read when one asks.
fn room_ceiling() -> ConsumerCeiling {
    let room = crate::server::rtc::room_config::load_room_config();
    ConsumerCeiling {
        tools: room.allowed_tools,
        skills: room.allowed_skills,
        models: room.shared_models,
    }
}

#[cfg(test)]
mod tests {
    use super::{person_policy, turn_policy, TurnBy};
    use crate::engine::session_policy::SessionPolicy;
    use crate::server::people::People;

    fn installed() -> Vec<String> {
        vec!["music".into(), "cfo".into(), "game".into()]
    }

    fn household() -> People {
        let mut people = People::default();
        let person = crate::server::person::asking(Some("u-friend"));
        let set = |v: serde_json::Value| v.as_object().cloned().unwrap();
        people
            .set(&person, &set(serde_json::json!({"apps": ["music", "game"], "models": ["deepseek-flash"]})))
            .unwrap();
        people.keep_for_owner(vec!["game".into()]);
        people
    }

    /// The engine's own turns and the export are the owner's; so is a
    /// request typed as the owner's, or by the owner's own phone.
    #[test]
    fn the_engine_the_export_and_the_owner_run_under_the_owners_policy() {
        for by in [TurnBy::Engine, TurnBy::Export] {
            assert_eq!(turn_policy(by), SessionPolicy::owner(), "{by:?}");
        }
        let people = household();
        let owners_phone = crate::server::person::Person::test_owner().wire();
        assert_eq!(person_policy("owner", None, &installed(), &people), SessionPolicy::owner());
        assert_eq!(person_policy("paired", Some(&owners_phone), &installed(), &people), SessionPolicy::owner());
    }

    /// A paired phone that is not the owner's, and a request that names no
    /// one, run under a ceiling of apps and of this Mac's models — never the
    /// owner's policy; their tools stay with their level.
    #[test]
    fn a_person_who_is_not_the_owner_is_held_to_their_own_ceiling() {
        let people = household();
        let friend = person_policy("paired", Some("u-friend"), &installed(), &people);
        assert_eq!(friend.allowed_models, Some(vec!["deepseek-flash".to_string()]));
        let skills = friend.allowed_skills.clone().expect("apps are held");
        assert_eq!(skills.len(), 1, "{skills:?}");
        assert!(skills.contains("music"), "game is the owner's alone, cfo is not theirs");
        assert!(friend.allowed_tools.is_none() && !friend.locked, "tools are the level's, not the policy's");
        // Nobody set anything: every app but the owner's, no model.
        let stranger = person_policy("paired", Some("u-stranger"), &installed(), &people);
        assert_eq!(stranger.allowed_models, Some(vec![]));
        assert_eq!(stranger.allowed_skills.map(|s| s.len()), Some(2));
        // A request that names no one is no one: the defaults, not the owner.
        for user_type in ["paired", "pairing", ""] {
            let nobody = person_policy(user_type, None, &installed(), &people);
            assert_eq!(nobody.allowed_models, Some(vec![]), "{user_type:?}");
        }
    }

    /// A room's consumer is under the room's ceiling, its own kind of
    /// policy (locked, the consumer's prompt), as before.
    #[test]
    fn a_room_consumer_is_under_the_rooms_ceiling() {
        let room = person_policy("consumer", Some("u-friend"), &installed(), &household());
        assert!(room.locked);
        assert!(
            room.allowed_tools.is_some() && room.allowed_skills.is_some() && room.allowed_models.is_some()
        );
    }
}

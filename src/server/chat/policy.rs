//! Which session policy a turn runs under: the one place that decides what
//! every turn that is not the owner's is held to — a room guest by the
//! room's config, any other person by their own settings
//! (`server::people`), the owner by nothing. Every turn asks
//! [`turn_policy`] (lock:
//! `structure_lock_tests::which_turn_gets_the_owner_policy_is_decided_in_one_place`).
//!
//! A household turn uses the same decision at both doors (`person_turn`):
//! the current settings, no more than any level stamped on the request.

use crate::engine::permission::PermissionMode;
use crate::engine::session_policy::{ConsumerCeiling, SessionPolicy};
use crate::engine::AgentEngine;
use crate::server::people::{self, People, Refusal};
use crate::server::person::Person;
use crate::server::ServerState;

/// Who stands behind a turn, as far as its policy goes.
#[derive(Debug, Clone, Copy)]
pub(crate) enum TurnBy<'a> {
    /// A person as resolved by the typed or task-end door, with one reading
    /// of the household's settings and the names of the installed apps.
    Person {
        person: &'a Person,
        people: &'a People,
        installed: &'a [String],
    },
    Consumer,
    /// A typed turn from the owner, including their own paired devices.
    Owner,
    /// The engine's own turns, including task ends in the owner's sessions.
    Engine,
    /// The prompt export: what the owner's turn at the page's place sends.
    Export,
}

/// The policy a turn of `by` runs under.
pub(crate) fn turn_policy(by: TurnBy<'_>) -> SessionPolicy {
    match by {
        TurnBy::Person { person, people, installed } if !person.is_owner() => {
            let ceiling = people.ceiling_of(person, installed);
            SessionPolicy::person(ceiling.skills, ceiling.models)
        }
        TurnBy::Consumer => SessionPolicy::consumer(room_ceiling()),
        TurnBy::Person { .. } | TurnBy::Owner | TurnBy::Engine | TurnBy::Export => {
            SessionPolicy::owner()
        }
    }
}

/// A household turn admitted under its person's current settings. Its use
/// is charged to the same person that the decision was made for.
pub(crate) struct PersonTurn {
    person: Person,
    pub policy: SessionPolicy,
    pub level: PermissionMode,
}

impl PersonTurn {
    pub(crate) async fn count(&self, state: &ServerState, engine: &mut AgentEngine) {
        let (usage, _) = engine.take_spend();
        people::record_turn(state, &self.person, (usage.prompt + usage.output) as u64).await;
    }
}

/// Decide whether a person's turn can run, and what it runs under. A typed
/// turn cannot exceed the device's stamped level; an unattended task end
/// has no device stamp. Both read the current settings when taking the turn.
pub(crate) async fn person_turn(
    state: &ServerState,
    person: Person,
    device_level: Option<PermissionMode>,
) -> Result<PersonTurn, Refusal> {
    let installed: Vec<_> = people::installed(&state.skills)
        .await
        .into_iter()
        .map(|s| s.name)
        .collect();
    let people = people::current();
    if let Some(why) = people::turn_refusal_with(state, &person, &people).await {
        return Err(why);
    }
    let policy = turn_policy(TurnBy::Person {
        person: &person,
        people: &people,
        installed: &installed,
    });
    let level = people.level_of(&person);
    Ok(PersonTurn {
        person,
        policy,
        level: device_level.map_or(level, |stamp| stamp.min(level)),
    })
}

/// An unattended or already queued turn has no HTTP reply. Keep its refusal
/// as a system row, with the same fact the typed door returns immediately.
pub(crate) async fn keep_refusal(state: &ServerState, session: &str, agent: &str, why: Refusal) {
    let status = why.status();
    let line = match why {
        Refusal::NoModel => format!("No shared model ({status})"),
        Refusal::OverLimit => format!("Daily token limit reached ({status})"),
    };
    super::helpers::persist_and_emit_with_images(
        &state.manager.global_sessions,
        &state.events_tx,
        agent,
        ("system", "user", &line),
        Some(session),
        false,
        None,
        (None, None),
        &[],
        false,
    )
    .await;
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
    use super::{turn_policy, TurnBy};
    use crate::engine::session_policy::SessionPolicy;
    use crate::server::people::People;

    fn policy_for(id: Option<&str>, people: &People) -> SessionPolicy {
        turn_policy(TurnBy::Person {
            person: &crate::server::person::asking(id),
            people,
            installed: &installed(),
        })
    }

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
        for by in [TurnBy::Engine, TurnBy::Export, TurnBy::Owner] {
            assert_eq!(turn_policy(by), SessionPolicy::owner(), "{by:?}");
        }
        let owners_phone = crate::server::person::Person::test_owner().wire();
        assert_eq!(policy_for(Some(&owners_phone), &household()), SessionPolicy::owner());
    }

    /// A paired phone that is not the owner's, and a request that names no
    /// one, run under a ceiling of apps and of this Mac's models — never the
    /// owner's policy; their tools stay with their level.
    #[test]
    fn a_person_who_is_not_the_owner_is_held_to_their_own_ceiling() {
        let people = household();
        let friend = policy_for(Some("u-friend"), &people);
        // A share of a retired Linggen Cloud id also shares the model it became.
        assert_eq!(friend.allowed_models, Some(vec!["deepseek-flash".to_string(), "deepseek-v4.1-flash".to_string()]));
        let skills = friend.allowed_skills.clone().expect("apps are held");
        assert_eq!(skills.len(), 1, "{skills:?}");
        assert!(skills.contains("music"), "game is the owner's alone, cfo is not theirs");
        assert!(friend.allowed_tools.is_none() && !friend.locked, "tools are the level's, not the policy's");
        // Nobody set anything: every app but the owner's, no model.
        let stranger = policy_for(Some("u-stranger"), &people);
        assert_eq!(stranger.allowed_models, Some(vec![]));
        assert_eq!(stranger.allowed_skills.map(|s| s.len()), Some(2));
        // A request that names no one is no one: the defaults, not the owner.
        assert_eq!(policy_for(None, &people).allowed_models, Some(vec![]));
    }

    /// A room's consumer is under the room's ceiling, its own kind of
    /// policy (locked, the consumer's prompt), as before.
    #[test]
    fn a_room_consumer_is_under_the_rooms_ceiling() {
        let room = turn_policy(TurnBy::Consumer);
        assert!(room.locked);
        assert!(
            room.allowed_tools.is_some() && room.allowed_skills.is_some() && room.allowed_models.is_some()
        );
    }
}

//! Which session policy a turn runs under: the one place that decides which
//! turn gets the owner's policy (no tool or skill ceiling, the owner's
//! prompt) and which gets a room's ceiling. Every turn asks
//! [`turn_policy`] (lock:
//! `structure_lock_tests::which_turn_gets_the_owner_policy_is_decided_in_one_place`).
//!
//! What a person may do beyond this — their permission level — is their
//! standing's (`person::Standing`), not the policy's.

use crate::engine::session_policy::{ConsumerCeiling, SessionPolicy};

/// Who stands behind a turn, as far as its policy goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TurnBy<'a> {
    /// A person's request, of the wire's `user_type`: a room's consumer runs
    /// under the room's ceiling; the owner, a paired device and anyone
    /// else under the owner's policy.
    Person(&'a str),
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
        TurnBy::Person(user_type) => SessionPolicy::from_user_type(user_type, room_ceiling),
        TurnBy::Engine | TurnBy::Export => SessionPolicy::owner(),
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
    use super::{turn_policy, TurnBy};
    use crate::engine::session_policy::SessionPolicy;

    /// Only a room consumer's own request is capped; every other turn is
    /// the owner's, as each site chose it before this one place did.
    #[test]
    fn only_a_room_consumers_request_is_capped() {
        for by in [
            TurnBy::Person("owner"),
            TurnBy::Person("paired"),
            TurnBy::Person("pairing"),
            TurnBy::Person(""),
            TurnBy::Engine,
            TurnBy::Export,
        ] {
            assert_eq!(turn_policy(by), SessionPolicy::owner(), "{by:?}");
        }
        let room = turn_policy(TurnBy::Person("consumer"));
        assert!(room.locked);
        assert!(
            room.allowed_tools.is_some()
                && room.allowed_skills.is_some()
                && room.allowed_models.is_some()
        );
    }
}

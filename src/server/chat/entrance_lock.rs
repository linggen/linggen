//! R4 pins that are facts about source (a door's calls), and the lock on who
//! may begin a turn. The behaviour pins are in `tests/system/turn_entrance.rs`
//! and the engine's `turn_place::entrance` tests.

use crate::server::lock_scan::production_code;

/// Files whose production code holds `needle`, with a count each.
fn found(needle: &str) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = production_code(&["src", "crates/linggen-engine/src"])
        .into_iter()
        // This file names what it scans for.
        .filter(|(rel, _)| rel != "src/server/chat/entrance_lock.rs")
        .map(|(rel, code)| (rel, code.matches(needle).count()))
        .filter(|(_, n)| *n > 0)
        .collect();
    out.sort();
    out
}

/// R4 pin (b), recall half: her recall tuning (`tune_companion`) is called at
/// her own door (`ready_for_turn`), by a companion's chat turn and by the
/// prompt export — and by none of the doors that tell her a turn through the
/// shared core (`tasks.rs`, `triggers.rs`' delivery, `mcp_agent.rs`). Not
/// observable on the fake model (it needs the embedding model), so it is
/// pinned as the calls. A turn told to her runs on the engine's recall as it
/// was left.
#[test]
fn her_recall_is_tuned_at_her_door_alone() {
    let sites: Vec<String> = found("tune_companion(").into_iter().map(|(rel, _)| rel).collect();
    assert_eq!(
        sites,
        [
            "src/server/chat/admin.rs",
            "src/server/chat/handler.rs",
            "src/server/resident/turn.rs",
        ]
    );
}

/// R4 pin (b), delivery half: a message to a chat agent (`deliver_to_chat_agent`)
/// is reached from `on_agent_chat`'s non-`YINYUE_AGENT` branch and from its own
/// resend, which carries the same `to` — never with her address.
#[test]
fn the_delivery_door_is_called_only_for_an_address_that_is_not_hers() {
    let calls: Vec<(String, usize)> = found("deliver_to_chat_agent(");
    // The definition, the one call in `on_agent_chat`'s else branch, and the
    // resend closure's call.
    assert_eq!(calls, [("src/server/resident/triggers.rs".to_string(), 3)]);
    let triggers = production_code(&["src/server/resident"])
        .into_iter()
        .find(|(rel, _)| rel == "src/server/resident/triggers.rs")
        .map(|(_, code)| code)
        .unwrap();
    let route = triggers.split("fn on_agent_chat(").nth(1).unwrap();
    let her = route.find("if to == YINYUE_AGENT").expect("her address is routed first");
    let delivered = route.find("deliver_to_chat_agent(").expect("the delivery");
    assert!(her < delivered, "her address is sent to her own door before any delivery");
}

const ENTRANCE: &str = "crates/linggen-engine/src/engine/turn_place/entrance.rs";

/// Where a turn is set up outside the entrance, and why: each is a thing
/// that is not a session turn. None of them calls `open_session_turn`.
const OUTSIDE_THE_ENTRANCE: &[(&str, &str)] = &[
    (
        "src/server/chat/light.rs",
        "a light reply: no tools and no loop; `reply_now` begins a turn for its language alone",
    ),
    (
        "crates/linggen-engine/src/engine/tools/delegation.rs",
        "a Task child: a fresh engine its parent builds and drops, never a session's",
    ),
    (
        "src/server/chat/admin.rs",
        "the prompt export: sets an engine up, runs no loop",
    ),
];

/// Doors that start the agent loop in a turn they open themselves, through
/// the entrance. (`handler.rs` opens the turn of the doors that share
/// `run_session_turn`.)
const OPEN_THEIR_OWN_TURN: &[&str] = &[
    "src/server/chat/plan_flow.rs",
    "crates/linggen-engine/src/extensions/missions/scheduler.rs",
    "src/server/mcp_agent.rs",
];

/// Files that start the loop inside a turn `run_session_turn` opened: they
/// never open one (a second opening would put back what the first set).
const INSIDE_AN_OPENED_TURN: &[&str] = &[
    "src/server/chat/cloud_gate.rs",
    "src/server/chat/runtime.rs",
    "src/server/chat/skill_dispatch.rs",
    "src/server/chat/structured.rs",
];

fn files_with(needles: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = production_code(&["src", "crates/linggen-engine/src"])
        .into_iter()
        .filter(|(rel, _)| !rel.ends_with("tests.rs") && rel != "src/server/chat/entrance_lock.rs")
        .filter(|(_, code)| needles.iter().any(|n| code.contains(n)))
        .map(|(rel, _)| rel)
        .collect();
    out.sort();
    out
}

/// A turn is begun in one place — the entrance — and by the light reply.
/// A new caller of `begin_turn` fails here until it goes through
/// `open_session_turn` or is listed with its reason.
#[test]
fn a_turn_is_begun_by_the_entrance_and_nowhere_else() {
    let mut allowed: Vec<&str> = vec![ENTRANCE, "src/server/chat/light.rs"];
    allowed.sort();
    assert_eq!(files_with(&[concat!(".begin", "_turn(")]), allowed);
}

/// Every door that starts the loop opens its turn through the entrance, or
/// is listed with its reason: a new door that starts the loop fails here.
#[test]
fn a_door_that_starts_the_loop_opens_its_turn_through_the_entrance() {
    let mut loops = // Written in two halves: the loop lock (`turn_place/lock_tests.rs`) counts
    // the text, and this file is not a caller.
    files_with(&[concat!(".run_agent", "_loop("), concat!("run_loop_with", "_tracking(")]);
    let mut listed: Vec<&str> = OPEN_THEIR_OWN_TURN
        .iter()
        .chain(INSIDE_AN_OPENED_TURN)
        .copied()
        .chain(std::iter::once("crates/linggen-engine/src/engine/tools/delegation.rs"))
        .collect();
    loops.sort();
    listed.sort();
    assert_eq!(loops, listed, "a file starts the loop that no list names");
    let opening = files_with(&["open_session_turn("]);
    let mut want: Vec<&str> = OPEN_THEIR_OWN_TURN.to_vec();
    want.extend(["src/server/chat/handler.rs", ENTRANCE]);
    want.sort();
    assert_eq!(opening, want, "who opens a session turn");
    for (door, why) in OUTSIDE_THE_ENTRANCE {
        assert!(!why.is_empty() && !opening.iter().any(|f| f == door), "{door} stays outside: {why}");
    }
}

/// Who chooses a session turn's model (`run_session_turn`'s `ModelBy`): a
/// person's chat and her turn choose it themselves before the turn; a task's
/// end (told to an agent that is not hers) and a delivered `agent_chat`
/// have the member's kept model chosen for them by the turn core. Pinned as
/// the calls: a delivery cannot be reached on a rebuilt engine from a system
/// test (with no engine alive its message lands in a session of its own).
#[test]
fn a_task_end_and_a_delivery_have_the_members_kept_model_chosen_for_them() {
    assert_eq!(
        files_with(&["ModelBy::TheMembersKeptModel"]),
        // The turn core (which chooses it) and the two doors that ask for it.
        ["src/server/chat/handler.rs", "src/server/resident/triggers.rs", "src/server/tasks.rs"]
    );
    assert_eq!(
        files_with(&["ModelBy::TheDoorBeforeTheTurn"]),
        ["src/server/chat/handler.rs", "src/server/resident/turn.rs", "src/server/tasks.rs"]
    );
}

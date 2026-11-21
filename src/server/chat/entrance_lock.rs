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


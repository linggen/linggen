//! The door's `memory_notes` tool: Linggen's own notes (`own_memory`) for a
//! host outside Linggen. The ops, their arguments and answers are
//! `own_memory::call`, the same a Linggen session's tool uses; this side
//! says who may use it (the owner, at this Mac, when `outside_agents` is
//! v2), who the caller is (a coding host: never shown what the fence keeps
//! back), and which host wrote.

use super::{outside_mode, tool_content, McpDeps};
use crate::config::OutsideAgents;
use crate::own_memory::{self, Audience, Caller, Store};
use serde_json::Value;
use std::path::PathBuf;

const ONLY_AT_THIS_MAC: &str = "memory_notes answers only the owner, at this Mac";
const NOT_SERVED_IN_V1: &str = "Linggen's own notes are not served to outside agents on this Mac (the v1 way is on): keep your memory the way your instructions say";

pub(super) async fn run(deps: &McpDeps<'_>, op: &'static str, args: Value) -> Value {
    let Some(store) = Store::this_machine() else {
        return tool_content("this machine has no home folder to keep notes in".to_string(), true);
    };
    let now = crate::util::now_ts_secs() as i64;
    match answer(&store, outside_mode(deps).await, deps.at_this_mac, op, &args, now) {
        Ok(data) => tool_content(data.to_string(), false),
        Err(why) => tool_content(why, true),
    }
}

/// One op's answer for a host at the door.
fn answer(store: &Store, mode: OutsideAgents, at_this_mac: bool, op: &str, args: &Value, now: i64) -> Result<Value, String> {
    if !at_this_mac {
        return Err(ONLY_AT_THIS_MAC.to_string());
    }
    if mode != OutsideAgents::V2 {
        return Err(NOT_SERVED_IN_V1.to_string());
    }
    let text = |key: &str| args.get(key).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
    let caller = Caller {
        // A host on the door is always a coding host.
        audience: Audience::Work,
        source: text("host").unwrap_or("mcp").to_string(),
        cwd: text("cwd").map(PathBuf::from),
    };
    own_memory::call(store, &caller, op, args, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outside_memory::Env;
    use serde_json::json;

    const NOW: i64 = 1_795_608_000;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let store = Store::new(home.join(".linggen/memory"), Env::new(&home));
        (dir, store)
    }

    #[test]
    fn v1_and_a_remote_caller_get_nothing() {
        let (_d, store) = store();
        let a = json!({"scope": "person", "summary": "Likes tea", "kind": "person"});
        assert!(answer(&store, OutsideAgents::V1, true, "write", &a, NOW).unwrap_err().contains("not served"));
        assert!(answer(&store, OutsideAgents::V2, false, "write", &a, NOW).unwrap_err().contains("only the owner"));
        assert!(!store.root().exists(), "nothing was written");
    }

    #[test]
    fn a_host_writes_as_itself_and_never_reaches_the_fenced_notes() {
        let (_d, store) = store();
        let wrote = answer(
            &store,
            OutsideAgents::V2,
            true,
            "write",
            &json!({"scope": "project:/work/app", "topic": "build", "summary": "Use just test", "kind": "rule", "host": "codex"}),
            NOW,
        )
        .unwrap();
        let read = answer(&store, OutsideAgents::V2, true, "read", &json!({"id": wrote["id"]}), NOW).unwrap();
        assert_eq!(read["source"], "codex");
        // Yinyue's mind note exists; the door can neither write one nor see it.
        let hers = Caller { audience: Audience::Companion, source: "yinyue".into(), cwd: None };
        let mind = json!({"scope": "area:mind", "topic": "past", "summary": "A hard year", "kind": "person"});
        let mine = own_memory::call(&store, &hers, "write", &mind, NOW).unwrap();
        assert!(answer(&store, OutsideAgents::V2, true, "write", &mind, NOW).is_err());
        assert!(answer(&store, OutsideAgents::V2, true, "read", &json!({"id": mine["id"]}), NOW).is_err());
        let hits = answer(&store, OutsideAgents::V2, true, "search", &json!({"words": "hard year"}), NOW).unwrap();
        assert_eq!(hits["hits"], json!([]));
        let index = answer(&store, OutsideAgents::V2, true, "index", &json!({"cwd": "/work/app"}), NOW).unwrap();
        assert!(index["index"].as_str().unwrap().contains("Use just test"));
        assert!(!index["index"].as_str().unwrap().contains("hard year"));
    }
}

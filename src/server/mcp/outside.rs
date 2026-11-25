//! The door's `memory_agents` tool: what other agents remember, read from
//! their own files and never written (`outside_memory`). The registry, the
//! confinement and the redaction are there; this side reads the arguments,
//! says which way (`v1` / `v2`) this Linggen serves outside agents, and
//! words the answer. `op=index` is the session-start block: under v2 it
//! carries Linggen's own notes too, put together in
//! `own_memory::forward`.
//!
//! For the owner at this Mac alone: another machine's agent has its own
//! memory there, and these are this Mac's files.

use super::catalog::OutsideOp;
use super::{tool_content, McpDeps};
use crate::config::OutsideAgents;
use crate::own_memory::{forward, Store};
use crate::outside_memory::{self as outside, Env};
use serde_json::{json, Value};
use std::path::Path;

const ONLY_AT_THIS_MAC: &str = "memory_agents answers only the owner, at this Mac";
const DEFAULT_HITS: usize = 10;
const MOST_HITS: usize = 30;

pub(super) async fn run(deps: &McpDeps<'_>, op: OutsideOp, args: Value) -> Value {
    if !deps.at_this_mac {
        return tool_content(ONLY_AT_THIS_MAC.to_string(), true);
    }
    let mode = super::outside_mode(deps).await;
    let Some(env) = Env::this_machine() else {
        return tool_content("this machine has no home folder to read memories from".to_string(), true);
    };
    let store = Store::this_machine();
    match answer(&env, store.as_ref(), mode, op, &args) {
        Ok(data) => tool_content(data.to_string(), false),
        Err(why) => tool_content(why, true),
    }
}

/// One op's answer, from what is on disk under `env` (and, for the index
/// under v2, Linggen's own notes in `store`).
fn answer(env: &Env, store: Option<&Store>, mode: OutsideAgents, op: OutsideOp, args: &Value) -> Result<Value, String> {
    let text = |key: &str| args.get(key).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
    match op {
        OutsideOp::Mode => Ok(json!({ "outside_agents": mode.word() })),
        OutsideOp::Index => {
            let host = text("host").and_then(|word| outside::host_agent(env, word));
            let own = store.filter(|_| mode == OutsideAgents::V2);
            let now = crate::util::now_ts_secs() as i64;
            let start = forward::session_start(env, own, host.as_ref(), text("cwd").map(Path::new), now);
            let slice = &start.others;
            let lines: Vec<Value> = slice
                .entries()
                .into_iter()
                .map(|(source, agent, about, line)| {
                    json!({ "source": source, "agent": agent.label(), "about": about, "text": line.as_str() })
                })
                .collect();
            let notes: Vec<Value> = slice
                .notes
                .iter()
                .map(|n| json!({ "source": n.source, "state": n.state }))
                .collect();
            Ok(json!({
                "outside_agents": mode.word(),
                "block": start.block,
                "own_notes": start.own.shown,
                "own_left_out": start.own.left_out,
                "lines": lines,
                "left_out": slice.cut,
                "skipped": notes,
            }))
        }
        OutsideOp::Read => {
            let path = text("path").ok_or("name the note's `path`")?;
            let note = outside::read(env, path).map_err(|e| e.to_string())?;
            Ok(json!({
                "source": note.source,
                "path": env.tilde(&note.path),
                "truncated": note.truncated,
                "text": note.text.as_str(),
            }))
        }
        OutsideOp::Search => {
            let words = text("words").ok_or("name the `words` to find")?;
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .map_or(DEFAULT_HITS, |n| (n as usize).clamp(1, MOST_HITS));
            let hits: Vec<Value> = outside::search(env, words, limit)
                .iter()
                .map(|h| json!({ "source": h.source, "path": env.tilde(&h.path), "line": h.line, "text": h.text.as_str() }))
                .collect();
            Ok(json!({ "hits": hits }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn mode_says_which_way_this_linggen_serves() {
        let h = home();
        let env = Env::new(h.path());
        let got = answer(&env, None, OutsideAgents::V2, OutsideOp::Mode, &json!({})).unwrap();
        assert_eq!(got, json!({ "outside_agents": "v2" }));
        let got = answer(&env, None, OutsideAgents::V1, OutsideOp::Mode, &json!({})).unwrap();
        assert_eq!(got, json!({ "outside_agents": "v1" }));
    }

    #[test]
    fn an_index_with_nothing_to_forward_is_an_empty_block_not_an_error() {
        let h = home();
        let env = Env::new(h.path());
        let got = answer(&env, None, OutsideAgents::V2, OutsideOp::Index, &json!({"cwd": "/x/y", "host": "claude-code"})).unwrap();
        assert_eq!(got["block"], "");
        assert_eq!(got["lines"], json!([]));
    }

    #[test]
    fn read_and_search_name_what_they_need() {
        let h = home();
        let env = Env::new(h.path());
        assert!(answer(&env, None, OutsideAgents::V2, OutsideOp::Read, &json!({})).is_err());
        assert!(answer(&env, None, OutsideAgents::V2, OutsideOp::Search, &json!({"words": "  "})).is_err());
    }
}

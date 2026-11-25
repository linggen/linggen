//! Linggen reads what other agents remember and forwards it, through the
//! MCP door's `memory_agents` tool (doc/mcp-spec.md § memory_agents), and
//! `[memory] outside_agents` says whether outside hosts get that (v2) or
//! today's save-and-recall (v1, the way back; v2 is the default since 3.0).
//!
//! The folders are made-up fixtures in the world's own home.

use crate::support::{Setup, World};
use serde_json::{json, Value};
use std::path::Path;

fn rpc(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
}

/// One `memory_agents` call: its answer as JSON, or the error text.
async fn agents(w: &World, args: Value) -> Result<Value, String> {
    let res = w
        .api
        .post("/mcp", rpc("tools/call", json!({"name": "memory_agents", "arguments": args})))
        .await;
    let text = res["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("{res}"))
        .to_string();
    match res["result"]["isError"] == json!(true) {
        true => Err(text),
        false => Ok(serde_json::from_str(&text).unwrap_or_else(|_| panic!("{text}"))),
    }
}

async fn server_text(w: &World) -> String {
    let init = w.api.post("/mcp", rpc("initialize", json!({}))).await;
    init["result"]["instructions"].as_str().unwrap().to_string()
}

/// Claude Code's memory for a project folder of the world's home, as
/// Claude Code lays it out. Returns the memory folder.
fn claude_memory(home: &Path, project: &Path) -> std::path::PathBuf {
    let encoded: String = project
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let folder = home.join(".claude/projects").join(encoded).join("memory");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("MEMORY.md"),
        "# Notes\n- [Ship rule](ship_rule.md) — ships only on Fridays\n",
    )
    .unwrap();
    std::fs::write(folder.join("ship_rule.md"), "Ship only on Fridays, after the gate.\n").unwrap();
    folder
}

const V2: Option<&str> = Some("\n[memory]\nlinggen = \"v1\"\noutside_agents = \"v2\"\n");
const V1: Option<&str> = Some("\n[memory]\nlinggen = \"v1\"\noutside_agents = \"v1\"\n");

/// v1 in the config (the way back). The door says so, its text is the old
/// one, and the index and the read still work when asked for.
#[tokio::test]
async fn with_v1_outside_hosts_are_served_the_old_way_and_the_tool_still_reads() {
    let w = World::start(Setup { config_append: V1, ..Setup::default() }).await;
    assert_eq!(agents(&w, json!({"op": "mode"})).await.unwrap()["outside_agents"], "v1");
    assert!(server_text(&w).await.contains("Core is tiny"));

    let home = w.home.home();
    let project = home.join("work");
    std::fs::create_dir_all(&project).unwrap();
    let folder = claude_memory(&home, &project);

    // Codex is forwarded what Claude Code remembers; Claude Code, nothing of its own.
    let args = json!({"op": "index", "cwd": project.display().to_string(), "host": "codex"});
    let got = agents(&w, args).await.unwrap();
    let block = got["block"].as_str().unwrap();
    assert!(block.contains("Ship rule") && block.contains("Claude Code"), "{block}");
    let args = json!({"op": "index", "cwd": project.display().to_string(), "host": "claude-code"});
    assert_eq!(agents(&w, args).await.unwrap()["block"], "");

    let note = folder.join("ship_rule.md").display().to_string();
    let read = agents(&w, json!({"op": "read", "path": note})).await.unwrap();
    assert!(read["text"].as_str().unwrap().contains("only on Fridays"));
    let hits = agents(&w, json!({"op": "search", "words": "fridays gate"})).await.unwrap();
    assert!(hits["hits"].as_array().unwrap().iter().any(|h| h["line"] == 1), "{hits}");

    // Outside every source: refused, whether the file is there or not.
    std::fs::write(home.join("diary.md"), "private").unwrap();
    for path in [home.join("diary.md"), home.join("nothing.md")] {
        let refused = agents(&w, json!({"op": "read", "path": path.display().to_string()})).await;
        assert!(refused.unwrap_err().contains("not a note inside"), "{path:?}");
    }
}

/// Nothing written in the config: the door says v2.
#[tokio::test]
async fn by_default_the_door_says_v2() {
    let w = World::start(Setup { memory_defaults: true, ..Setup::default() }).await;
    assert_eq!(agents(&w, json!({"op": "mode"})).await.unwrap()["outside_agents"], "v2");
}

/// v2: the door says so, the server text no longer asks a host to save into
/// Linggen's store, and a settings save from a page that does not know the
/// switch leaves it as it was.
#[tokio::test]
async fn v2_is_what_the_door_says_and_a_settings_save_keeps_it() {
    let w = World::start(Setup { config_append: V2, ..Setup::default() }).await;
    assert_eq!(agents(&w, json!({"op": "mode"})).await.unwrap()["outside_agents"], "v2");
    let text = server_text(&w).await;
    assert!(text.contains("memory_agents op=index") && !text.contains("Core is tiny"), "{text}");
    assert!(text.chars().count() <= 2048);

    let mut config = w.api.get("/api/config").await;
    config.as_object_mut().unwrap().remove("memory");
    w.api.post("/api/config", config).await;
    assert_eq!(agents(&w, json!({"op": "mode"})).await.unwrap()["outside_agents"], "v2");
}

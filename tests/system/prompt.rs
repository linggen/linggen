//! What each member is handed before a turn: the system-prompt export, its
//! tools, the memory blocks — and what a session withholds.

use crate::support::memory::embedder_available;
use crate::support::model::Agent;
use crate::support::rows::calls_by;
use crate::support::{snap_text, text, tool, Setup, World};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn tool_names(export: &Value) -> Vec<String> {
    let tools = export["tools"].as_array().cloned().unwrap_or_default();
    tools
        .iter()
        .filter_map(|t| t["function"]["name"].as_str().or(t["name"].as_str()))
        .map(str::to_string)
        .collect()
}

fn has(names: &[String], name: &str) -> bool {
    names.iter().any(|n| n == name)
}

fn memory_tools(names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter(|n| n.starts_with("mcp__memory__"))
        .cloned()
        .collect()
}

/// The export of each member at a skill table, snapshotted; Ling's lists the
/// skill's own tools, Yinyue's only what her place names.
#[tokio::test]
async fn export_per_member_at_a_skill_table() {
    let w = World::start(Setup::default()).await;
    let sid = w
        .api
        .create_session(json!({"title": "table", "skill": "dice"}))
        .await;
    for (who, model) in [("ling", "fake-ling"), ("yinyue", "fake-yinyue")] {
        let e = w.api.export(&sid, who, "").await;
        assert_eq!(
            (e["provider"].as_str(), e["model_id"].as_str()),
            (Some("openai"), Some(model))
        );
        let mut names = tool_names(&e);
        names.sort();
        let prompt = e["system_prompt"].as_str().unwrap_or("");
        snap_text(
            &format!("export_{who}"),
            &w,
            &format!("tools: {names:?}\n\n{prompt}"),
        );
    }
    let his = tool_names(&w.api.export(&sid, "ling", "").await);
    assert!(
        has(&his, "Look") && has(&his, "Roll"),
        "Ling's export lacks the skill's tools: {his:?}"
    );
    // `pet: true` is hers alone, the lead's set included.
    assert!(!has(&his, "Peek"), "Ling is offered her pet tool: {his:?}");
    let hers = tool_names(&w.api.export(&sid, "yinyue", "").await);
    assert!(has(&hers, "Peek"), "her place's tool is missing: {hers:?}");
    for not in ["Look", "Roll", "AskUser"] {
        assert!(!has(&hers, not), "she is offered {not}: {hers:?}");
    }
    w.assert_all_scripted();
}

/// An ordinary chat's export once the memory server's tools are in (the MCP
/// connection is made in the background after startup).
async fn export_with_memory_tools(w: &World, sid: &str) -> Vec<String> {
    let start = Instant::now();
    loop {
        let names = tool_names(&w.api.export(sid, "ling", &w.work()).await);
        if !memory_tools(&names).is_empty() {
            return names;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "memory tools never arrived: {names:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// `withheld_tools: [mcp__memory]` — no member is offered a memory tool, the
/// model's call carries none, and a call made anyway is refused.
#[tokio::test]
async fn session_withheld_tools_reach_no_member() {
    let w = World::start(Setup::default().script(
        Agent::Ling,
        vec![
            tool(
                "mcp__memory__memory_add",
                json!({"content": "Alex likes test rows.", "tier": "semantic"}),
            ),
            text("Kept nothing."),
        ],
    ))
    .await;
    let root = w.work();
    let control = w
        .api
        .create_session(json!({"title": "control", "project_root": root}))
        .await;
    export_with_memory_tools(&w, &control).await;

    let sid = w
        .api
        .create_session(
            json!({"title": "scratch", "project_root": root, "withheld_tools": ["mcp__memory"]}),
        )
        .await;
    for who in ["ling", "yinyue"] {
        let names = tool_names(&w.api.export(&sid, who, &root).await);
        assert!(
            memory_tools(&names).is_empty(),
            "{who} is offered {:?}",
            memory_tools(&names)
        );
    }

    w.turn(&sid, &root, "Remember that I like test rows.", Agent::Ling)
        .await;
    let offered = w.model.calls_of(Agent::Ling)[0].tool_names();
    assert!(
        memory_tools(&offered).is_empty(),
        "the model call offered {:?}",
        memory_tools(&offered)
    );
    // The call made anyway: refused, and the refusal is what the model reads.
    let after = &w.model.calls_of(Agent::Ling)[1];
    let (role, refusal) = after.thread().pop().expect("the call's result");
    assert_eq!(role, "tool");
    snap_text("withheld_call_result", &w, &refusal);
    // Saved like any other call: its call row and the refusal as its result,
    // so the thread and the history show what the model read.
    let rows = w.rows(&sid);
    assert!(
        calls_by(&rows, "ling").contains(&"mcp__memory__memory_add".to_string()),
        "the withheld call left no call row: {rows:?}"
    );
    assert!(
        rows.iter()
            .any(|r| r.is_observation && r.content.contains("reason=withheld")),
        "the refusal left no result row: {rows:?}"
    );
    let history = w.api.history(&sid, &root).await.to_string();
    assert!(
        history.contains("mcp__memory__memory_add"),
        "the history does not show the withheld call"
    );
    let stored = w.memory.call("list", json!({"limit": 10})).await;
    assert_eq!(
        stored.as_array().map_or(0, Vec::len),
        0,
        "a row reached the store: {stored}"
    );
    w.assert_all_scripted();
}

/// Memory in the prompt, on a seeded store (the real embedding model): a
/// `$HOME` session gets core rows only; a project session gets the
/// candidates line and the index, never a sibling project's rows.
///
/// Ignored by default (nextest lists it as skipped): it needs the model in
/// `~/.cache/huggingface`. `./scripts/check.sh system` runs it when the model
/// is there, or when `LINGGEN_SYSTEM_REQUIRE_EMBED=1` — then a missing model
/// fails it instead of passing on nothing.
#[tokio::test]
#[ignore = "needs the embedding model (Qwen3-Embedding-0.6B in ~/.cache/huggingface)"]
async fn memory_core_candidates_and_index_in_the_prompt() {
    if !embedder_available() {
        let required = std::env::var_os("LINGGEN_SYSTEM_REQUIRE_EMBED").is_some_and(|v| v == "1");
        assert!(
            !required,
            "LINGGEN_SYSTEM_REQUIRE_EMBED=1 but no embedding model in ~/.cache/huggingface"
        );
        eprintln!("SKIPPED (passes on nothing): no embedding model in ~/.cache/huggingface");
        return;
    }
    let started = Instant::now();
    let w = World::start(Setup {
        embedder: true,
        memory_rows: Some("alex.json"),
        ..Setup::default()
    })
    .await;
    eprintln!("world with a seeded store: {:?}", started.elapsed());
    let home = w.home.home().display().to_string();
    let harbor = w.home.work().join("harbor");
    std::fs::create_dir_all(harbor.join(".git")).expect("harbor repo");
    let harbor = harbor.display().to_string();

    let at_home = w
        .api
        .create_session(json!({"title": "home", "project_root": home}))
        .await;
    let prompt = export_prompt(&w, &at_home, &home).await;
    assert!(
        prompt.contains("## Core memory — who the user is"),
        "no core block:\n{prompt}"
    );
    assert!(
        prompt.contains("Alex lives in Lisbon"),
        "the core row is missing"
    );
    assert!(
        !prompt.contains("Memory scopes here: "),
        "a $HOME session got the candidates line"
    );
    assert!(
        !prompt.contains("## Index — "),
        "a $HOME session got an index"
    );

    let project = w
        .api
        .create_session(json!({"title": "harbor", "project_root": harbor}))
        .await;
    let prompt = export_prompt(&w, &project, &harbor).await;
    let candidates = prompt
        .lines()
        .find(|l| l.starts_with("Memory scopes here: "));
    let candidates = candidates.unwrap_or_else(|| panic!("no candidates line:\n{prompt}"));
    assert!(
        candidates.contains("harbor"),
        "the project is not a candidate: {candidates}"
    );
    assert!(prompt.contains("## Index — "), "no index heading");
    assert!(
        prompt.contains("Harbor: tabs, not spaces"),
        "the project's indexed row is missing"
    );
    assert!(
        prompt.contains("Commit to main, never branch"),
        "the workspace's indexed row is missing"
    );
    assert!(
        !prompt.contains("Lantern"),
        "a sibling project's row leaked in"
    );
    w.assert_all_scripted();
    eprintln!("memory test total: {:?}", started.elapsed());
}

async fn export_prompt(w: &World, sid: &str, root: &str) -> String {
    let e = w.api.export(sid, "ling", root).await;
    e["system_prompt"].as_str().unwrap_or("").to_string()
}

/// A project walk never reads above `$HOME`: a repo and a CLAUDE.md in the
/// folder that holds the home stay out of every prompt (export and live
/// turn); a repo under home still brings its own.
#[tokio::test]
async fn the_project_walk_stops_at_home() {
    let w = World::start(Setup::default().script(Agent::Ling, vec![text("Here.")])).await;
    // Made after the world's outside-git guard: the tree that holds HOME.
    let above = &w.home.root;
    std::fs::create_dir_all(above.join(".git")).expect("a repo above home");
    std::fs::write(above.join("CLAUDE.md"), "ABOVE-HOME rules.").expect("its CLAUDE.md");
    let harbor = w.home.work().join("harbor");
    std::fs::create_dir_all(harbor.join(".git")).expect("harbor repo");
    std::fs::write(harbor.join("CLAUDE.md"), "Harbor: tabs only.").expect("harbor CLAUDE.md");
    let chat = w.home.work().join("chat");
    std::fs::create_dir_all(&chat).expect("chat folder");
    let (harbor, chat) = (harbor.display().to_string(), chat.display().to_string());

    let in_chat = w
        .api
        .create_session(json!({"title": "chat", "project_root": chat}))
        .await;
    let prompt = export_prompt(&w, &in_chat, &chat).await;
    assert!(!prompt.contains("ABOVE-HOME"), "read above home:\n{prompt}");

    let in_harbor = w
        .api
        .create_session(json!({"title": "harbor", "project_root": harbor}))
        .await;
    let prompt = export_prompt(&w, &in_harbor, &harbor).await;
    assert!(
        prompt.contains("Harbor: tabs only."),
        "the repo's CLAUDE.md is missing"
    );
    assert!(!prompt.contains("ABOVE-HOME"), "read above home:\n{prompt}");

    w.turn(&in_chat, &chat, "Where are we?", Agent::Ling).await;
    let call = w.model.calls_of(Agent::Ling).pop().expect("his call");
    assert!(
        !call.text().contains("ABOVE-HOME"),
        "a live turn read above home"
    );
    w.assert_all_scripted();
}

/// The Environment block names the zone `TZ` sets (the world runs on
/// `TZ=UTC`), not the machine's own.
#[tokio::test]
async fn the_environment_block_honours_tz() {
    let w = World::start(Setup::default()).await;
    let root = w.work();
    let sid = w
        .api
        .create_session(json!({"title": "chat", "project_root": root}))
        .await;
    let prompt = export_prompt(&w, &sid, &root).await;
    let line = prompt.lines().find(|l| l.starts_with("- Timezone: "));
    assert_eq!(line, Some("- Timezone: UTC"), "{prompt}");
    w.assert_all_scripted();
}

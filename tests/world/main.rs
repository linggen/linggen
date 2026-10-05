//! One hermetic world as a standalone process — the web UI tests
//! (tests/e2e, Playwright) start theirs through this, so both suites share
//! one world-builder (`tests/system/support`).
//!
//!   cargo test --test world --no-run      build it (and the `ling` it runs)
//!   <world binary> '<scenario JSON>'      start a world
//!
//! The scenario says what the fake model answers per member, and which
//! fixture home to start from:
//!
//!   {"fixture": "home",
//!    "scripts": {"ling":   [{"text": "Hi."}],
//!                "yinyue": [{"tool": "Write", "args": {...}}, {"text": "…"}]}}
//!
//! Once up it prints one JSON line — `{"url", "root", "work", "linggen_home"}`
//! — and then reads commands on stdin, one per line:
//!   `calls`  → one JSON line: every model call (agent, matched, model)
//!   `keep`   → keep the world's root when it stops (a failed test)
//! End of stdin stops the world: every process stopped, the root removed.
//! Run with no argument (as `cargo test` does) it does nothing.

#![allow(dead_code)]

#[path = "../system/support/mod.rs"]
mod support;

use serde_json::{json, Value};
use std::io::{BufRead, Write};
use support::model::{Agent, Reply};
use support::{Setup, World};

fn main() {
    let Some(arg) = std::env::args().nth(1).filter(|a| a.starts_with('{')) else {
        return;
    };
    let scenario: Value = serde_json::from_str(&arg).expect("scenario JSON");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(serve(setup(&scenario)));
}

fn setup(scenario: &Value) -> Setup {
    let mut setup = Setup {
        fixture: scenario["fixture"].as_str().map(leak),
        ..Setup::default()
    };
    let scripts = scenario["scripts"].as_object().cloned().unwrap_or_default();
    for (who, replies) in scripts {
        let agent = match who.as_str() {
            "ling" => Agent::Ling,
            "yinyue" => Agent::Yinyue,
            other => panic!("no member {other} in the fake model"),
        };
        let replies = replies.as_array().cloned().unwrap_or_default();
        setup = setup.script(agent, replies.iter().map(reply).collect());
    }
    setup
}

fn reply(v: &Value) -> Reply {
    if let Some(t) = v["text"].as_str() {
        return support::text(t);
    }
    let name = v["tool"].as_str().expect("a reply is {text} or {tool, args}");
    support::tool(name, v["args"].clone())
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

async fn serve(setup: Setup) {
    let w = World::start(setup).await;
    let hello = json!({
        "url": w.engine.base_url(),
        "root": w.home.root,
        "work": w.work(),
        "linggen_home": w.home.linggen_home(),
    });
    say(&hello);

    // Stdin is read on its own thread; the world stays on this runtime.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    while let Some(line) = rx.recv().await {
        match line.trim() {
            "calls" => say(&calls(&w)),
            // Home's Drop reads this: keep the root and print where.
            "keep" => std::env::set_var("LINGGEN_SYSTEM_KEEP", "1"),
            _ => {}
        }
    }
    drop(w);
}

fn calls(w: &World) -> Value {
    let calls: Vec<Value> = w
        .model
        .calls()
        .iter()
        .map(|c| {
            json!({
                "agent": c.scenario,
                "matched": c.matched,
                "model": c.model(),
                "path": c.path,
            })
        })
        .collect();
    json!({ "calls": calls })
}

fn say(v: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

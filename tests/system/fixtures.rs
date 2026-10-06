//! The fixture world itself: it loads as written, and the guard refuses
//! anything that would reach the person's own Linggen.

use crate::support::home::Home;
use crate::support::memory::Memory;
use crate::support::{guard, Setup, World};
use serde_json::{json, Value};
use std::path::Path;

fn ids(list: &Value, key: &str) -> Vec<String> {
    let items = list.as_array().cloned().unwrap_or_default();
    items
        .iter()
        .filter_map(|i| i[key].as_str().map(str::to_string))
        .collect()
}

#[tokio::test]
async fn the_fixture_home_loads() {
    let w = World::start(Setup::default()).await;
    let models = ids(&w.api.get("/api/models").await, "id");
    for m in ["fake-ling", "fake-yinyue", "fake-sol"] {
        assert!(
            models.contains(&m.to_string()),
            "model {m} missing: {models:?}"
        );
    }
    let skills = w.api.get("/api/skills").await.to_string();
    assert!(
        skills.contains("\"dice\"") && skills.contains("\"quiet\""),
        "skills: {skills}"
    );
    let missions = w.api.get("/api/missions").await.to_string();
    assert!(missions.contains("table-check"), "missions: {missions}");
    assert!(
        !missions.contains("\"dream\""),
        "a built-in mission was seeded into the fixture home"
    );
    w.assert_all_scripted();
}

/// A true first install — no config folder at all — boots on the engine's
/// own defaults, and its memory goes to the world's ling-mem (named only by
/// `LING_MEM_URL`), not the real one on 9528.
#[tokio::test]
async fn a_first_install_with_no_config_reaches_its_own_memory() {
    let w = World::start(Setup {
        no_config: true,
        ..Setup::default()
    })
    .await;
    let config = w.api.get("/api/config").await;
    assert_eq!(config["agent"]["ling_mem_url"], w.memory.url, "{config}");

    // A memory call through the engine lands in the world's own store.
    let note = format!("no-config world {}", w.home.root.display());
    let added = w
        .api
        .post(
            "/api/memory/issue_add",
            json!({"kind": "subject", "row_ids": ["r-no-config"], "note": note}),
        )
        .await;
    assert_eq!(added["ok"], json!(true), "{added}");
    let issues = w.memory.call("issues", json!({})).await.to_string();
    assert!(
        issues.contains(&note),
        "not in the world's ling-mem: {issues}"
    );

    assert!(
        !w.home.linggen_home().join("config").exists(),
        "booting wrote a config"
    );
    w.assert_all_scripted();
}

#[test]
#[should_panic(expected = "HERMETIC GUARD")]
fn guard_refuses_the_real_memory_url_in_env() {
    guard::mem_url("http://127.0.0.1:9528");
}

#[test]
#[should_panic(expected = "HERMETIC GUARD")]
fn guard_refuses_the_real_engine_port() {
    guard::port(9527, "a test");
}

#[test]
#[should_panic(expected = "HERMETIC GUARD")]
fn guard_refuses_the_real_memory_port_in_config() {
    guard::config_text("ling_mem_url = \"http://127.0.0.1:9528\"");
}

#[test]
#[should_panic(expected = "HERMETIC GUARD")]
fn guard_refuses_the_real_linggen_home() {
    let real = dirs::home_dir().expect("home").join(".linggen/sessions");
    guard::path_in_root(&real, Path::new("/"), "a test path");
}

#[test]
#[should_panic(expected = "HERMETIC GUARD")]
fn guard_refuses_a_real_port_in_any_home_file() {
    // A plain temp dir: a refused world's root would be kept.
    let home = tempfile::tempdir().expect("a scratch home");
    let skill = home.path().join(".linggen/skills/dice/notes.md");
    std::fs::create_dir_all(skill.parent().expect("dir")).expect("skill dir");
    std::fs::write(&skill, "curl http://127.0.0.1:9528/api/health").expect("write");
    guard::home_files(home.path(), &home.path().join(".cache/huggingface"));
}

#[test]
#[should_panic(expected = "HERMETIC GUARD")]
fn guard_refuses_a_real_port_after_the_run() {
    guard::after_run("the engine log", "mcp connect http://localhost:9528/mcp");
}

#[test]
#[should_panic(expected = "HERMETIC GUARD")]
fn guard_refuses_a_root_inside_a_git_repo() {
    guard::outside_git(Path::new(env!("CARGO_MANIFEST_DIR")));
}

/// A port someone else holds — and answers health on — is never taken for
/// our server: the child moves to a fresh port.
#[tokio::test]
async fn a_taken_port_is_never_mistaken_for_ours() {
    let stranger = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a stranger");
    let taken = stranger.local_addr().expect("stranger addr").port();
    std::thread::spawn(move || answer_healthy(stranger));
    let home = Home::new();
    let memory = Memory::start_on(&home, false, Some(taken)).await;
    assert!(
        !memory.url.ends_with(&format!(":{taken}")),
        "took the stranger's port {taken} for ours"
    );
}

/// A server that says 200 to anything.
fn answer_healthy(listener: std::net::TcpListener) {
    use std::io::{Read, Write};
    for mut conn in listener.incoming().flatten() {
        let mut buf = [0u8; 1024];
        let _ = conn.read(&mut buf);
        let _ =
            conn.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }
}

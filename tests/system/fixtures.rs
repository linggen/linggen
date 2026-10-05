//! The fixture world itself: it loads as written, and the guard refuses
//! anything that would reach the person's own Linggen.

use crate::support::{guard, Setup, World};
use serde_json::Value;
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
fn guard_refuses_a_root_inside_a_git_repo() {
    guard::outside_git(Path::new(env!("CARGO_MANIFEST_DIR")));
}

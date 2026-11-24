//! Settings → Notes over HTTP: what Linggen remembers, by source. Linggen's
//! own notes are read, edited, marked wrong and right, kept from being
//! raised, and deleted by their id; what other agents keep is listed and
//! read, and no route can change it. The notes are made-up fixtures in the
//! world's own home.

use crate::support::{Setup, World};
use serde_json::{json, Value};

const PERSON: &str = "---\ntopic: About\nscope: person\n---\n\n## Name is Alex\n<!-- id: p000000001 · kind: person · voice: user · created: 2026-11-20 · updated: 2026-11-20 · source: yinyue -->\nSaid so himself.\n";
const MIND: &str = "---\ntopic: Past\nscope: area:mind\n---\n\n## A hard year, closed now\n<!-- id: m000000001 · kind: person · voice: agent · created: 2026-11-20 · updated: 2026-11-20 · source: yinyue -->\nFelt low; concluded to rest more.\n";
const OLD: &str = "---\ntopic: Build\nscope: project:/work/app\n---\n\n## Use just test\n<!-- id: r000000001 · kind: rule · voice: agent · created: 2025-01-01 · updated: 2025-01-01 · source: codex -->\nNever cargo by hand.\n\n## Old wrong thing\n<!-- id: r000000002 · kind: gotcha · voice: agent · created: 2026-11-01 · updated: 2026-11-01 · source: codex · void: it was a test -->\n";

const CC_INDEX: &str = "# Notes\n\n- [Build](build.md) — how to build\n";
const CC_NOTE: &str = "Use just test.\n";
const CODEX_NOTE: &str = "A codex memory.\n";

/// The agents' files, as the test finds them again to prove nothing touched them.
struct Theirs(Vec<(std::path::PathBuf, &'static str)>);

fn seed(home: &crate::support::home::Home) -> Theirs {
    let memory = home.linggen_home().join("memory");
    for (file, text) in [("person/about.md", PERSON), ("areas/mind/past.md", MIND), ("projects/-work-app/build.md", OLD)] {
        let path = memory.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let claude = home.home().join(".claude/projects/-work-app/memory");
    let codex = home.home().join(".codex/memories");
    let theirs = vec![
        (claude.join("MEMORY.md"), CC_INDEX),
        (claude.join("build.md"), CC_NOTE),
        (codex.join("summary.md"), CODEX_NOTE),
    ];
    for (path, text) in &theirs {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    Theirs(theirs)
}

async fn world() -> (World, Theirs) {
    let w = World::start(Setup::default()).await;
    let theirs = seed(&w.home);
    (w, theirs)
}

fn ids(list: &Value, key: &str) -> Vec<String> {
    list[key].as_array().unwrap().iter().map(|v| v["id"].as_str().unwrap().to_string()).collect()
}

/// The sources are Linggen and each agent the registry reads, with counts;
/// each source's topics, entries and text can be read.
#[tokio::test]
async fn the_page_lists_each_source_and_reads_it() {
    let (w, _theirs) = world().await;
    let sources = w.api.get("/api/remembered-sources").await;
    assert_eq!(ids(&sources, "sources"), ["linggen", "claude-code", "codex"]);
    let own = &sources["sources"][0];
    assert_eq!((own["own"].clone(), own["count"].clone(), own["voided"].clone()), (json!(true), json!(4), json!(1)));
    assert_eq!(sources["sources"][1]["count"], 2);
    assert_eq!(sources["sources"][2]["count"], 1);

    let topics = w.api.get("/api/remembered-sources/linggen/topics").await;
    let paths: Vec<&str> = topics["topics"].as_array().unwrap().iter().map(|t| t["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["areas/mind/past.md", "person/about.md", "projects/-work-app/build.md"]);
    let notes = w.api.get("/api/remembered-sources/linggen/entries?topic=areas%2Fmind%2Fpast.md").await;
    assert_eq!(notes["entries"][0]["body"], "Felt low; concluded to rest more.");
    let one = w.api.get("/api/remembered-sources/linggen/note?id=p000000001").await;
    assert_eq!((one["summary"].clone(), one["voice"].clone()), (json!("Name is Alex"), json!("user")));

    let claude = w.api.get("/api/remembered-sources/claude-code/topics").await;
    let place = claude["topics"][0]["path"].as_str().unwrap().to_string();
    assert!(place.starts_with("~/.claude/projects/"), "{place}");
    let files = w.api.get(&format!("/api/remembered-sources/claude-code/entries?topic={}", place.replace('/', "%2F"))).await;
    let build = files["entries"].as_array().unwrap().iter().find(|f| f["name"] == "build.md").expect("build.md");
    let text = w.api.get(&format!("/api/remembered-sources/claude-code/note?path={}", build["path"].as_str().unwrap().replace('/', "%2F"))).await;
    assert_eq!(text["text"], "Use just test.\n");
    // Reading a source that does not exist, or a file that is not in it, is refused.
    assert_eq!(w.api.call("GET", "/api/remembered-sources/nobody/topics").await.0, 404);
    assert_eq!(w.api.call("GET", "/api/remembered-sources/codex/note?path=%2Fetc%2Fhosts").await.0, 404);
    assert_eq!(w.api.call("GET", "/api/remembered-sources/linggen/entries?topic=..%2Fx.md").await.0, 404);
}

/// An edit, a mark and a delete land in the files by the one writer, signed
/// as Linggen and as the person's; the list says the same after.
#[tokio::test]
async fn the_owner_tends_linggens_own_notes() {
    let (w, _theirs) = world().await;
    let file = w.home.linggen_home().join("memory/projects/-work-app/build.md");

    let (status, done) = w.api.put_json("/api/remembered-notes/r000000001", json!({"summary": "Use just test, always", "body": "Never cargo by hand, never."})).await;
    assert_eq!(status, 200, "{done}");
    assert_eq!(done["note"]["summary"], "Use just test, always");
    assert!(std::fs::read_to_string(&file).unwrap().contains("## Use just test, always"));

    // Un-void brings the wrong note back; void marks it again, with a reason.
    let (_, back) = w.api.post_with_status("/api/remembered-notes/r000000002/unvoid", json!({})).await;
    assert_eq!(back["note"]["void"], Value::Null);
    let (_, again) = w.api.post_with_status("/api/remembered-notes/r000000002/void", json!({"reason": "still wrong"})).await;
    assert_eq!(again["note"]["void"], "still wrong");
    let (status, none) = w.api.post_with_status("/api/remembered-notes/r000000002/void", json!({"reason": " "})).await;
    assert_eq!((status, none["error"].clone()), (422, json!("empty")));

    let (_, raised) = w.api.post_with_status("/api/remembered-notes/p000000001/no-raise", json!({"on": true})).await;
    assert_eq!(raised["note"]["no_raise"], true);
    assert!(std::fs::read_to_string(w.home.linggen_home().join("memory/person/about.md")).unwrap().contains("no-raise"));

    // A status line is not memory: refused with a code, nothing changed.
    let (status, refused) = w.api.put_json("/api/remembered-notes/r000000001", json!({"summary": "Status: the migration is done"})).await;
    assert_eq!((status, refused["error"].clone()), (422, json!("status")));
    assert!(w.api.get("/api/remembered-sources/linggen/note?id=r000000001").await["summary"].as_str().unwrap().starts_with("Use just test"));

    let (status, gone) = w.api.call("DELETE", "/api/remembered-notes/p000000001").await;
    assert_eq!((status, gone["deleted"].clone()), (200, json!("p000000001")));
    assert!(!w.home.linggen_home().join("memory/person/about.md").exists());
    let history = std::fs::read_dir(w.home.linggen_home().join("memory/.state/history/person__about.md")).unwrap().count();
    assert!(history >= 1, "the file's last version is kept");
    assert_eq!(w.api.call("DELETE", "/api/remembered-notes/p000000001").await.0, 404);
}

/// A save from a page that showed an old version of a note that changed is a
/// conflict, with a code the page puts in its own words.
#[tokio::test]
async fn an_edit_of_a_note_that_changed_since_is_a_conflict() {
    let (w, _theirs) = world().await;
    let shown = w.api.get("/api/remembered-sources/linggen/note?id=r000000001").await;
    w.api.put_json("/api/remembered-notes/r000000001", json!({"body": "someone else"})).await;
    let (status, refused) = w.api.put_json("/api/remembered-notes/r000000001", json!({"body": "mine", "base": shown["version"]})).await;
    assert_eq!((status, refused["error"].clone()), (409, json!("conflict")));
}

/// Review: the one marked wrong, and the one not read for 90 days. Opening
/// the page counts no use, so the unused one stays unused.
#[tokio::test]
async fn the_review_lists_void_and_unused_and_looking_counts_nothing() {
    let (w, _theirs) = world().await;
    for _ in 0..2 {
        let review = w.api.get("/api/remembered-notes/review").await;
        let rows: Vec<(String, String)> = review["review"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r["id"].as_str().unwrap().to_string(), r["why"].as_str().unwrap().to_string()))
            .collect();
        assert_eq!(rows, [("r000000002".to_string(), "void".to_string()), ("r000000001".to_string(), "unused".to_string())]);
        w.api.get("/api/remembered-sources/linggen/note?id=r000000001").await;
        w.api.get("/api/remembered-sources/linggen/entries?topic=projects%2F-work-app%2Fbuild.md").await;
    }
    assert!(!w.home.linggen_home().join("memory/.state/uses.jsonl").exists());
}

/// Other agents' memory is read only. No route changes it: every write
/// method on a source route is refused; the routes that change a note know
/// only Linggen's own notes, so an agent's file name or path is no note; and
/// after all of it their files are the bytes they were.
#[tokio::test]
async fn another_agents_memory_cannot_be_changed() {
    let (w, theirs) = world().await;
    let place = w.api.get("/api/remembered-sources/claude-code/topics").await["topics"][0]["path"].as_str().unwrap().to_string();
    let enc = |s: &str| s.replace('/', "%2F");
    let routes = [
        "/api/remembered-sources".to_string(),
        "/api/remembered-sources/claude-code/topics".to_string(),
        "/api/remembered-sources/codex/topics".to_string(),
        format!("/api/remembered-sources/claude-code/entries?topic={}", enc(&place)),
        format!("/api/remembered-sources/claude-code/note?path={}", enc(&format!("{place}/build.md"))),
    ];
    for route in &routes {
        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            let (status, _) = w.api.call(method, route).await;
            assert_eq!(status, 405, "{method} {route}");
        }
    }
    // A write route given their file's name, path or what looks like an id.
    let wanted = ["MEMORY.md", "build.md", "summary.md", "~%2F.claude%2Fprojects%2F-work-app%2Fmemory%2Fbuild.md", "r000000009"];
    for id in wanted {
        let (put, _) = w.api.put_json(&format!("/api/remembered-notes/{id}"), json!({"body": "overwritten"})).await;
        let (del, _) = w.api.call("DELETE", &format!("/api/remembered-notes/{id}")).await;
        let (void, _) = w.api.post_with_status(&format!("/api/remembered-notes/{id}/void"), json!({"reason": "x"})).await;
        assert_eq!((put, del, void), (404, 404, 404), "{id}");
    }
    for (path, text) in &theirs.0 {
        assert_eq!(std::fs::read_to_string(path).unwrap(), *text, "{} changed", path.display());
    }
    assert_eq!(std::fs::read_dir(path_of(&theirs.0[0].0)).unwrap().count(), 2, "a file appeared or went");
}

fn path_of(file: &std::path::Path) -> &std::path::Path {
    file.parent().unwrap()
}

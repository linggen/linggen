//! The move from the v1 store: what is kept, the files it makes, a second
//! run, a dry run, and the start-up check. Made-up rows only.

use super::fence::Audience;
use super::find::{read, Read};
use super::format::{Kind, Scope, Topic, Voice};
use super::migrate::{migrate, upgrade_once, Report, Upgrade, V1Row};
use super::Store;
use crate::outside_memory::Env;
use serde_json::json;
use std::path::{Path, PathBuf};

const APP: &str = "/work/alex/app";

fn scratch() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let store = Store::new(home.join(".linggen/memory"), Env::new(&home));
    (dir, store)
}

fn row(id: &str, kind: &str, tier: &str, from: &str, scope: Option<&str>, summary: Option<&str>, content: &str) -> serde_json::Value {
    json!({
        "id": id, "type": kind, "tier": tier, "from": from, "scope": scope, "summary": summary,
        "content": content, "created_at": "2026-11-10T12:00:00Z", "host": "claude-code"
    })
}

/// A store of made-up rows: each reason to keep or to drop once.
fn fixture() -> Vec<V1Row> {
    let long = "x".repeat(4100);
    let rows = vec![
        row("r1", "preference", "semantic", "user", Some(APP), Some("Build: run just test before a push"), "Alex said: run just test before every push."),
        row("r2", "decision", "semantic", "derived", Some(APP), Some("Cache: use sqlite, not files"), "Why: one writer, atomic."),
        row("r3", "learned", "semantic", "derived", None, Some("GOTCHA: cp over a live binary exits silently"), "Swap with a new inode and mv."),
        row("r4", "fact", "core", "user", None, None, "Alex's cat Mimi is male. She was adopted in 2024."),
        row("r5", "fact", "semantic", "derived", Some(APP), Some("App: has three tabs"), "A project fact that git holds."),
        row("r6", "built", "semantic", "derived", Some(APP), Some("Shipped the tabs"), "Done."),
        row("r7", "fixed", "semantic", "derived", Some(APP), Some("Fixed the crash"), "Done."),
        row("r8", "tried", "semantic", "derived", Some(APP), Some("Tried X"), "No."),
        row("r9", "decision", "episodic", "derived", Some(APP), Some("Staged line"), "Not settled."),
        row("r10", "learned", "semantic", "derived", Some(APP), Some("Fixed: the tabs are now merged"), "It was done today."),
        row("r11", "learned", "semantic", "derived", Some(APP), Some("Key: the deploy token"), "token AKIAABCDEFGHIJKLMNOP is in the vault"),
        row("r12", "preference", "semantic", "derived", Some(APP), Some("Build: run just test before a push"), "Same words, another row."),
        row("r13", "decision", "semantic", "derived", Some(APP), Some("Long: a very long note"), &long),
        row("r14", "preference", "semantic", "user", Some("relative/dir"), Some("Odd: a scope that is no path"), "x"),
        json!({"id": "r15", "type": "preference", "tier": "semantic", "from": "user", "summary": "Theirs: a phone's row", "content": "x",
               "created_at": "2026-11-10T12:00:00Z", "account_id": "acct-2"}),
        json!({"id": "r16", "type": "preference", "tier": "semantic", "from": "user", "summary": "Old: replaced", "content": "x",
               "created_at": "2026-11-10T12:00:00Z", "expired_at": "2026-11-11T12:00:00Z"}),
        row("r17", "advice", "semantic", "derived", None, Some("A type nobody knows"), "x"),
    ];
    V1Row::from_list(&json!(rows)).0
}

fn files_of(store: &Store) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(items) = std::fs::read_dir(dir) else { return };
        for item in items.flatten() {
            let path = item.path();
            let skip = path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'));
            match (skip, path.is_dir()) {
                (true, _) => {}
                (false, true) => walk(&path, out),
                (false, false) => out.push(path),
            }
        }
    }
    let mut out = Vec::new();
    walk(store.root(), &mut out);
    out.sort();
    out
}

fn parsed(path: &Path) -> Topic {
    Topic::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn the_fixture_becomes_the_expected_topic_files() {
    let (_d, store) = scratch();
    let report = migrate(&store, &fixture(), true);
    assert_eq!(report.read, 17);
    assert_eq!(report.landed, 4, "{:?}", report.lines(false));
    assert_eq!(report.already, 0);
    let dropped = |reason: &str| report.dropped.get(reason).copied().unwrap_or(0);
    assert_eq!(dropped("what was done (built, fixed, tried)"), 3);
    assert_eq!(dropped("episodic staging"), 1);
    assert_eq!(dropped("a project fact (state or spec)"), 1);
    assert_eq!(dropped("reads as status"), 1);
    assert_eq!(dropped("holds a key-shaped secret"), 1);
    assert_eq!(dropped("same summary as a note in its topic"), 1);
    assert_eq!(dropped("body over its length limit"), 1);
    assert_eq!(dropped("a scope that is not a folder path"), 1);
    assert_eq!(dropped("another person's row"), 1);
    assert_eq!(dropped("already replaced in v1"), 1);
    assert_eq!(dropped("a type with no kind here"), 1);
    assert_eq!(report.dropped_total(), 13);
    assert_eq!(report.landed + report.dropped_total(), report.read);

    let root = store.root();
    let names: Vec<String> = files_of(&store).iter().map(|p| p.strip_prefix(root).unwrap().display().to_string()).collect();
    assert_eq!(
        names,
        [
            "person/alex-s.md",
            "person/cp-over.md",
            "projects/-work-alex-app/build.md",
            "projects/-work-alex-app/cache.md",
        ],
        "{names:?}"
    );
    let build = parsed(&root.join("projects/-work-alex-app/build.md"));
    assert_eq!(build.scope, Scope::Project(PathBuf::from(APP)));
    let entry = &build.entries[0];
    assert_eq!((entry.id.as_str(), entry.kind, entry.voice), ("r1", Kind::Rule, Voice::User));
    assert_eq!((entry.created.as_str(), entry.source.as_str()), ("2026-11-10", "claude-code"));
    assert_eq!(entry.body, "Alex said: run just test before every push.");
    let cat = parsed(&root.join("person/alex-s.md"));
    assert_eq!((cat.entries[0].kind, cat.entries[0].voice, cat.scope.clone()), (Kind::Person, Voice::User, Scope::Person));
    // The summary of a row without one is its first clause.
    assert_eq!(cat.entries[0].summary, "Alex's cat Mimi is male");
    // Found by its v1 id, as any note.
    let Read::Entry(found) = read(&store, Audience::Work, "r2", 0).unwrap() else { panic!("an id read a file") };
    assert_eq!(found.entry.kind, Kind::Decision);
}

#[test]
fn a_second_run_adds_nothing() {
    let (_d, store) = scratch();
    let first = migrate(&store, &fixture(), true);
    let before: Vec<_> = files_of(&store).iter().map(|p| (p.clone(), std::fs::read(p).unwrap())).collect();
    let again = migrate(&store, &fixture(), true);
    assert_eq!((again.landed, again.already), (0, first.landed));
    let after: Vec<_> = files_of(&store).iter().map(|p| (p.clone(), std::fs::read(p).unwrap())).collect();
    assert_eq!(before, after);
}

#[test]
fn a_dry_run_writes_nothing_and_counts_what_a_run_does() {
    let (_d, store) = scratch();
    let dry = migrate(&store, &fixture(), false);
    assert!(!store.root().exists(), "a dry run made {:?}", files_of(&store));
    let real = migrate(&store, &fixture(), true);
    assert_eq!(dry, real);
    assert!(dry.lines(true).last().unwrap().contains("nothing was written"));
}

#[test]
fn a_topic_past_its_limits_is_split_and_loses_no_note() {
    let (_d, store) = scratch();
    let rows: Vec<serde_json::Value> = (0..120)
        .map(|n| {
            let mut r = row(&format!("n{n:03}"), "decision", "semantic", "derived", Some(APP), Some(&format!("Cache: choice number {n}")), &"Why: a reason. ".repeat(30));
            r["created_at"] = json!(format!("2026-10-{:02}T12:00:00Z", 1 + n % 28));
            r
        })
        .collect();
    let rows = V1Row::from_list(&json!(rows)).0;
    let report = migrate(&store, &rows, true);
    assert_eq!(report.landed, 120);
    let files = files_of(&store);
    assert!(files.len() > 1, "{files:?}");
    let ids: usize = files.iter().map(|p| parsed(p).entries.len()).sum();
    assert_eq!(ids, 120);
    for path in &files {
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.lines().count() < super::format::FILE_LINES && text.len() <= super::format::FILE_BYTES, "{path:?}");
    }
    // The second run finds every note, in whichever file it went to.
    assert_eq!(migrate(&store, &rows, true).already, 120);
}

#[test]
fn the_topic_is_the_subject_of_the_summary() {
    let rows = |summary: &str| V1Row::from_list(&json!([row("t", "decision", "semantic", "derived", Some(APP), Some(summary), "body")])).0;
    for (summary, topic) in [
        ("Sanji params: delete redundant knobs", "sanji"),
        ("Hanli 2026-10-05: the new-user install test must run", "the-new-user"),
        ("GOTCHA — never run dart format", "never"),
        ("Remove Kokoro entirely; TTS is Qwen3", "remove"),
        ("Prices are not hand-kept", "prices"),
        ("图只在建造模式绘制，游玩时绝不现场生成", "图只在建造模式绘制"),
    ] {
        let (_d, store) = scratch();
        migrate(&store, &rows(summary), true);
        let files = files_of(&store);
        assert_eq!(files.len(), 1, "{summary}");
        let name = files[0].file_stem().unwrap().to_string_lossy().to_string();
        assert_eq!(name, topic, "{summary}");
    }
}

#[test]
fn rows_are_read_from_a_list_and_from_an_export_file() {
    let (rows, bad) = V1Row::from_list(&json!([row("a", "decision", "semantic", "user", None, None, "x"), {"no": "id"}, 7]));
    assert_eq!((rows.len(), bad), (1, 2));
    let text = format!("{}\n\n{}\nnot json\n", row("a", "decision", "semantic", "user", None, None, "x"), row("b", "fact", "core", "user", None, None, "y"));
    let (rows, bad) = V1Row::from_ndjson(&text);
    assert_eq!((rows.len(), bad), (2, 1));
}

fn never() -> Result<Vec<V1Row>, String> {
    panic!("the rows were asked for")
}

#[test]
fn the_upgrade_runs_once_and_records_it() {
    let (_d, store) = scratch();
    let Upgrade::Moved(report) = upgrade_once(&store, true, || Ok(fixture())).unwrap() else { panic!("not moved") };
    assert_eq!(report.landed, 4);
    assert!(store.root().join(".state/moved-from-v1.json").exists());
    let before = files_of(&store);
    assert_eq!(upgrade_once(&store, true, never).unwrap(), Upgrade::AlreadyLooked);
    assert_eq!(files_of(&store), before);
}

#[test]
fn explicit_v1_is_left_alone_and_records_nothing() {
    let (_d, store) = scratch();
    assert_eq!(upgrade_once(&store, false, never).unwrap(), Upgrade::NotWanted);
    assert!(!store.root().exists());
    // Going back to v2 later does the move then.
    assert!(matches!(upgrade_once(&store, true, || Ok(fixture())).unwrap(), Upgrade::Moved(_)));
}

#[test]
fn a_mac_that_already_has_notes_is_left_alone() {
    let (_d, store) = scratch();
    let hand_made = "---\ntopic: About\nscope: person\n---\n\n## Name is Alex\n<!-- id: p000000001 · kind: person · voice: user · created: 2026-11-20 · updated: 2026-11-20 · source: yinyue -->\n";
    let path = store.root().join("person/about.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, hand_made).unwrap();
    assert_eq!(upgrade_once(&store, true, never).unwrap(), Upgrade::HasNotes);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), hand_made);
    assert_eq!(files_of(&store).len(), 1);
    assert_eq!(upgrade_once(&store, true, never).unwrap(), Upgrade::AlreadyLooked);
}

#[test]
fn a_store_with_no_rows_or_rows_that_cannot_be_read_records_nothing() {
    let (_d, store) = scratch();
    assert_eq!(upgrade_once(&store, true, || Ok(Vec::new())).unwrap(), Upgrade::NothingToMove);
    assert!(upgrade_once(&store, true, || Err("memory is not reachable".to_string())).is_err());
    assert!(!store.root().join(".state/moved-from-v1.json").exists());
    assert!(matches!(upgrade_once(&store, true, || Ok(fixture())).unwrap(), Upgrade::Moved(_)));
}

#[test]
fn the_report_says_what_was_kept_and_why_the_rest_was_not() {
    let (_d, store) = scratch();
    let report: Report = migrate(&store, &fixture(), false);
    let lines = report.lines(true);
    assert!(lines[0].starts_with("17 v1 rows read"), "{lines:?}");
    assert!(lines[1].starts_with("would keep 4 in 4 topic files"), "{lines:?}");
    assert!(lines.iter().any(|l| l == "  episodic staging: 1"), "{lines:?}");
}

/// By hand, on a copy of a real store: `scripts/scratch-test.sh --lib
/// the_move_on_a_copy -- --ignored --nocapture`, with the export (`ling mem
/// export`, vectors dropped) saved as `.cache/v1-rows.ndjson` under the
/// scratch home. Moves into a scratch store, never the person's; prints
/// the counts and the ids kept, so a judged pass can be compared.
#[test]
#[ignore = "needs a copy of a v1 export in the scratch home"]
fn the_move_on_a_copy() {
    let home = std::env::var("HOME").unwrap();
    let text = std::fs::read_to_string(Path::new(&home).join(".cache/v1-rows.ndjson")).expect("a copy of the export");
    let (rows, unreadable) = V1Row::from_ndjson(&text);
    let (_d, store) = scratch();
    let dry = migrate(&store, &rows, false);
    let real = migrate(&store, &rows, true);
    assert_eq!(dry, real);
    for line in real.lines(false) {
        println!("{line}");
    }
    println!("unreadable {unreadable}");
    let files = files_of(&store);
    println!("topic files on disk {}", files.len());
    for path in files {
        for entry in parsed(&path).entries {
            println!("KEPT {}", entry.id);
        }
    }
    assert_eq!(migrate(&store, &rows, true).landed, 0);
}

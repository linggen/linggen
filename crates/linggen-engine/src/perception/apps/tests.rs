use super::declare::{check, usable, WorldLine, FIELDS_MAX};
use super::read::{self, App, Cache, LINES_MAX, LINE_CHARS, MAX_AGE_SECS, VALUE_CHARS};
use serde_json::json;
use std::time::{Duration, SystemTime};

fn decl(file: &str, fields: &[&str]) -> WorldLine {
    WorldLine {
        file: file.to_string(),
        fields: fields.iter().map(|f| f.to_string()).collect(),
    }
}

/// A skill folder with its data file written, modified `ago` seconds before
/// `now`.
fn app_with(dir: &std::path::Path, name: &str, body: &str, fields: &[&str], ago: u64) -> App {
    let skill = dir.join(name);
    std::fs::create_dir_all(skill.join("data")).unwrap();
    let file = skill.join("data/world.json");
    std::fs::write(&file, body).unwrap();
    set_age(&file, ago);
    App {
        name: name.to_string(),
        dir: skill,
        line: decl("data/world.json", fields),
    }
}

fn now() -> SystemTime {
    SystemTime::now()
}

fn set_age(file: &std::path::Path, ago: u64) {
    set_time(file, now() - Duration::from_secs(ago));
}

fn set_time(file: &std::path::Path, when: SystemTime) {
    std::fs::File::options()
        .write(true)
        .open(file)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

#[test]
fn a_valid_declaration_is_kept_as_written() {
    let raw = json!({"file": "data/world.json", "fields": ["songs", "playing"]});
    assert_eq!(
        check(raw).unwrap(),
        decl("data/world.json", &["songs", "playing"])
    );
}

#[test]
fn an_invalid_declaration_is_refused_with_its_reason() {
    let bad = [
        json!({"file": "../cfo/data/x.json", "fields": ["a"]}),
        json!({"file": "/etc/x.json", "fields": ["a"]}),
        json!({"file": "data/x.txt", "fields": ["a"]}),
        json!({"file": "data/x.json", "fields": []}),
        json!({"file": "data/x.json", "fields": ["Steps Today"]}),
        json!({"file": "data/x.json", "fields": ["a", "a"]}),
        json!({"file": "data/x.json"}),
        json!({"file": "data/x.json", "fields": ["a"], "run": "x.sh"}),
        json!("data/x.json"),
    ];
    for raw in bad {
        assert!(check(raw.clone()).is_err(), "{raw}");
    }
}

#[test]
fn too_many_fields_is_an_invalid_declaration() {
    let fields: Vec<String> = (0..=FIELDS_MAX).map(|i| format!("f{i}")).collect();
    let err = check(json!({"file": "data/x.json", "fields": fields})).unwrap_err();
    assert!(err.contains("fields"), "{err}");
    let fit: Vec<String> = (0..FIELDS_MAX).map(|i| format!("f{i}")).collect();
    assert!(check(json!({"file": "data/x.json", "fields": fit})).is_ok());
}

#[test]
fn only_an_installed_skill_may_declare_and_a_bad_one_is_dropped() {
    let raw = json!({"file": "data/w.json", "fields": ["a"]});
    assert!(usable(Some(raw.clone()), "notes", true).is_some());
    assert!(
        usable(Some(raw), "notes", false).is_none(),
        "a project copy"
    );
    assert!(usable(Some(json!({"file": 3})), "notes", true).is_none());
    assert!(usable(None, "notes", true).is_none());
}

#[test]
fn a_reading_becomes_one_line_of_label_value_pairs_and_its_age() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_with(
        dir.path(),
        "notes-demo",
        r#"{"notes": 12, "pinned_title": "Garden plan", "other": "not declared"}"#,
        &["notes", "pinned_title"],
        300,
    );
    let lines = read::lines(&[app], now(), &mut Cache::default());
    assert_eq!(
        lines,
        vec!["notes-demo: notes 12 · pinned title Garden plan · 5m ago"]
    );
}

#[test]
fn a_missing_file_or_one_with_no_declared_text_gives_no_line() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_with(dir.path(), "a", r#"{"x": 1}"#, &["x"], 0);
    app.line.file = "data/absent.json".to_string();
    assert!(read::read(&app, now(), &mut Cache::default()).is_none());

    let none = app_with(dir.path(), "b", r#"{"y": 1}"#, &["x"], 0);
    assert!(read::read(&none, now(), &mut Cache::default()).is_none());

    let not_json = app_with(dir.path(), "c", "steps: 12", &["steps"], 0);
    assert!(read::read(&not_json, now(), &mut Cache::default()).is_none());

    let array = app_with(dir.path(), "d", "[1, 2]", &["steps"], 0);
    assert!(read::read(&array, now(), &mut Cache::default()).is_none());
}

#[test]
fn a_stale_file_is_left_out_rather_than_shown_old() {
    let dir = tempfile::tempdir().unwrap();
    let fresh = app_with(
        dir.path(),
        "fresh",
        r#"{"n": 1}"#,
        &["n"],
        MAX_AGE_SECS - 60,
    );
    let stale = app_with(
        dir.path(),
        "stale",
        r#"{"n": 1}"#,
        &["n"],
        MAX_AGE_SECS + 60,
    );
    let lines = read::lines(&[fresh, stale], now(), &mut Cache::default());
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with("fresh: n 1 · 6d ago"), "{lines:?}");
}

#[test]
fn a_long_value_is_cut_and_a_long_line_drops_whole_fields() {
    let dir = tempfile::tempdir().unwrap();
    let long = "x".repeat(400);
    let body = json!({"a": long, "b": "y".repeat(40), "c": "z".repeat(40), "d": "w".repeat(40)});
    let app = app_with(
        dir.path(),
        "wide",
        &body.to_string(),
        &["a", "b", "c", "d"],
        0,
    );
    let line = read::lines(&[app], now(), &mut Cache::default()).remove(0);
    assert!(line.chars().count() <= LINE_CHARS, "{line}");
    assert!(
        line.contains(&format!("a {}…", "x".repeat(VALUE_CHARS))),
        "{line}"
    );
    assert!(!line.contains("· d "), "the last field went whole: {line}");
    assert!(line.ends_with("· just now"), "{line}");
}

#[test]
fn values_are_copied_as_text_and_never_interpreted() {
    let dir = tempfile::tempdir().unwrap();
    let body = r#"{"n": 6200, "flag": true, "s": "27 ms · usual 35", "obj": {"a": 1}, "list": null, "nl": "two\nlines"}"#;
    let app = app_with(
        dir.path(),
        "kinds",
        body,
        &["n", "flag", "s", "obj", "list", "nl"],
        0,
    );
    let line = read::lines(&[app], now(), &mut Cache::default()).remove(0);
    assert_eq!(
        line,
        "kinds: n 6200 · flag true · s 27 ms · usual 35 · nl two lines · just now"
    );
}

#[test]
fn an_unchanged_file_is_not_read_again_and_a_changed_one_is() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_with(
        dir.path(),
        "steps-demo",
        r#"{"steps": 1000}"#,
        &["steps"],
        120,
    );
    let file = app.dir.join("data/world.json");
    let at = now() - Duration::from_secs(120);
    set_time(&file, at);
    let mut cache = Cache::default();
    let first = read::lines(&[app.clone()], now(), &mut cache);
    assert_eq!(first, vec!["steps-demo: steps 1000 · 2m ago"]);

    // Same size, same time: the cached reading stands, the file is not parsed.
    std::fs::write(&file, r#"{"steps": 9999}"#).unwrap();
    set_time(&file, at);
    assert_eq!(read::lines(&[app.clone()], now(), &mut cache), first);

    // The time moves: read again.
    set_age(&file, 0);
    assert_eq!(
        read::lines(&[app], now(), &mut cache),
        vec!["steps-demo: steps 9999 · just now"]
    );
}

#[test]
fn over_the_line_limit_the_most_recently_changed_stay() {
    let dir = tempfile::tempdir().unwrap();
    let apps: Vec<App> = (0..LINES_MAX + 3)
        .map(|i| {
            app_with(
                dir.path(),
                &format!("app{i:02}"),
                r#"{"n": 1}"#,
                &["n"],
                600 * i as u64,
            )
        })
        .collect();
    let lines = read::lines(&apps, now(), &mut Cache::default());
    assert_eq!(lines.len(), LINES_MAX);
    assert!(lines[0].starts_with("app00:"), "newest first: {lines:?}");
    assert!(lines
        .last()
        .unwrap()
        .starts_with(&format!("app{:02}:", LINES_MAX - 1)));
}

#[test]
fn a_link_out_of_the_skill_folder_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let other = app_with(dir.path(), "other", r#"{"n": 1}"#, &["n"], 0);
    let skill = dir.path().join("prying");
    std::fs::create_dir_all(skill.join("data")).unwrap();
    std::os::unix::fs::symlink(
        other.dir.join("data/world.json"),
        skill.join("data/world.json"),
    )
    .unwrap();
    let app = App {
        name: "prying".into(),
        dir: skill,
        line: decl("data/world.json", &["n"]),
    };
    assert!(read::read(&app, now(), &mut Cache::default()).is_none());
}

#[test]
fn a_file_too_large_to_be_a_reading_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let body = json!({"n": 1, "pad": "p".repeat(read::FILE_BYTES_MAX as usize)});
    let app = app_with(dir.path(), "big", &body.to_string(), &["n"], 0);
    assert!(read::read(&app, now(), &mut Cache::default()).is_none());
}

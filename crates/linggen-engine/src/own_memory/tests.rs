//! Linggen's own memory on made-up notes in a scratch home: the file form,
//! the one writer, the index, the reads, the fence, and the locks that keep
//! them the only ones.

use super::fence::{audience_of, Audience};
use super::find::{read, search, Read, ReadError};
use super::format::{Area, Entry, Kind, Scope, Topic, Voice};
use super::index::{index, INDEX_BYTES, INDEX_LINES};
use super::write::{write, Action, Refusal, Write, STATUS_REFUSAL};
use super::{call, Caller, Store};
use crate::outside_memory::Env;
use serde_json::json;
use std::path::{Path, PathBuf};

/// 2026-11-24, noon UTC.
const NOW: i64 = 1_795_608_000;
const DAY: i64 = 24 * 3600;

fn scratch() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let store = Store::new(home.join(".linggen/memory"), Env::new(&home));
    (dir, store)
}

fn add_action(scope: Scope, topic: &str, summary: &str, kind: Kind, voice: Voice, body: &str) -> Action {
    Action::Add { scope, topic: topic.into(), summary: summary.into(), kind, voice, body: body.into() }
}

fn w(action: Action) -> Write {
    Write { action, user_directed: false, source: "linggen".into(), base: None }
}

/// Add a rule to `scope`; its id.
fn rule(store: &Store, scope: Scope, topic: &str, summary: &str, body: &str, at: i64) -> String {
    let action = add_action(scope, topic, summary, Kind::Rule, Voice::Agent, body);
    write(store, Audience::Companion, at, &w(action)).expect("a rule is written").id
}

fn person(store: &Store, summary: &str, at: i64) -> String {
    let action = add_action(Scope::Person, "about", summary, Kind::Person, Voice::User, "");
    write(store, Audience::Companion, at, &w(action)).unwrap().id
}

fn read_entry(store: &Store, audience: Audience, id: &str) -> super::find::Found {
    match read(store, audience, id, NOW).unwrap() {
        Read::Entry(found) => found,
        Read::File { .. } => panic!("an id read a file"),
    }
}

// ── the file form ──────────────────────────────────────────────────────────

#[test]
fn a_topic_survives_the_file_form() {
    let mut topic = Topic::new("Diet 饮食", Scope::Area(Area::Body));
    let mut a = Entry::new(
        "a1b2c3d4e5".into(),
        "早餐不吃糖",
        Kind::Rule,
        Voice::User,
        "2026-11-24",
        "yinyue",
        "Why: 血糖.\n\n## looks like an entry\nmore",
    );
    a.no_raise = true;
    a.extra.push(("phone".into(), "iphone".into()));
    let mut b = Entry::new("f6g7h8i9j0".into(), "Wrong note", Kind::Gotcha, Voice::Agent, "2026-11-23", "codex", "");
    b.void = Some("it was a test".into());
    topic.entries = vec![a, b];
    let text = topic.render();
    assert_eq!(Topic::parse(&text).unwrap(), topic);
    // A body heading is one level deeper, so it can never start an entry.
    assert_eq!(topic.entries[0].body, "Why: 血糖.\n\n### looks like an entry\nmore");
    assert_eq!(text.matches("\n## ").count(), 2);
}

#[test]
fn a_void_reason_cannot_break_the_meta_comment() {
    let (_d, store) = scratch();
    let id = rule(&store, Scope::Person, "x", "A rule", "", NOW);
    let action = Action::Void { id: id.clone(), reason: "wrong · really --> yes\nsecond line".into() };
    write(&store, Audience::Companion, NOW, &w(action)).unwrap();
    let found = read_entry(&store, Audience::Companion, &id);
    assert_eq!(found.entry.void.as_deref(), Some("wrong - really -> yes second line"));
}

#[test]
fn a_summary_is_eighty_characters_not_bytes() {
    let long: String = "记".repeat(120);
    let e = Entry::new("0123456789".into(), &long, Kind::Rule, Voice::Agent, "2026-11-24", "x", "");
    assert_eq!(e.summary.chars().count(), 80);
    let mut t = Topic::new("t", Scope::Person);
    t.entries.push(e.clone());
    assert_eq!(Topic::parse(&t.render()).unwrap().entries[0], e);
}

#[test]
fn a_file_that_is_not_a_topic_is_a_problem_not_a_crash() {
    let (_d, store) = scratch();
    rule(&store, Scope::Person, "ok", "Fine", "", NOW);
    std::fs::write(store.root().join("person/bad.md"), "just text\n").unwrap();
    let (seen, problems) = super::fence::seen(&store, Audience::Companion);
    assert_eq!(seen.len(), 1);
    assert_eq!(problems.len(), 1, "{problems:?}");
}

// ── the writer ─────────────────────────────────────────────────────────────

#[test]
fn a_note_is_added_read_by_id_and_by_path() {
    let (_d, store) = scratch();
    let id = rule(&store, Scope::Project(PathBuf::from("/work/app")), "Build", "Run just test", "Never cargo by hand.", NOW);
    assert_eq!(id.len(), 10);
    let found = read_entry(&store, Audience::Work, &id);
    assert_eq!(found.entry.body, "Never cargo by hand.");
    assert_eq!(found.rel, "projects/-work-app/build.md");
    let Read::File { entries, version, .. } = read(&store, Audience::Work, "projects/-work-app/build.md", NOW).unwrap() else {
        panic!("a path read an entry")
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(version, found.version);
}

#[test]
fn another_entry_changing_is_a_merge_and_the_same_entry_is_a_conflict() {
    let (_d, store) = scratch();
    let a = rule(&store, Scope::Person, "t", "Rule A", "first", NOW);
    let saw = read_entry(&store, Audience::Companion, &a).version;
    // Someone else adds B: the file's version moves on.
    rule(&store, Scope::Person, "t", "Rule B", "", NOW + 1);
    // My edit of A, from the old version: merged, B stays.
    let mut mine = w(Action::Update { id: a.clone(), summary: None, body: Some("second".into()), kind: None });
    mine.base = Some(saw.clone());
    let done = write(&store, Audience::Companion, NOW + 2, &mine).unwrap();
    assert!(done.merged);
    let Read::File { entries, .. } = read(&store, Audience::Companion, "person/t.md", NOW).unwrap() else {
        panic!("a path read an entry")
    };
    assert_eq!(entries.iter().map(|e| e.summary.as_str()).collect::<Vec<_>>(), ["Rule A", "Rule B"]);
    assert_eq!(entries[0].body, "second");
    // A has changed since the version I saw (by my own edit above); a second
    // edit from that same old version conflicts.
    let mut again = w(Action::Update { id: a.clone(), summary: None, body: Some("third".into()), kind: None });
    again.base = Some(saw);
    let refused = write(&store, Audience::Companion, NOW + 3, &again).unwrap_err();
    assert!(matches!(refused, Refusal::Conflict { .. }), "{refused:?}");
    // A version the history never held: read again.
    again.base = Some("0000000000000000".into());
    assert_eq!(write(&store, Audience::Companion, NOW + 3, &again).unwrap_err(), Refusal::ReadAgain);
}

#[test]
fn every_write_keeps_the_file_before_it() {
    let (_d, store) = scratch();
    let id = rule(&store, Scope::Person, "t", "Rule", "v1 text", NOW);
    let first = store.load("person/t.md").unwrap().unwrap().version;
    let edit = Action::Update { id, summary: None, body: Some("v2 text".into()), kind: None };
    write(&store, Audience::Companion, NOW + 1, &w(edit)).unwrap();
    let kept = store.at_version("person/t.md", &first).expect("the old file is kept");
    assert_eq!(kept.entries[0].body, "v1 text");
}

#[test]
fn a_topic_past_the_limits_is_split_and_every_note_is_still_found() {
    let (_d, store) = scratch();
    let body = "x".repeat(3000);
    let ids: Vec<String> = (0..12)
        .map(|n| rule(&store, Scope::Person, "big", &format!("Note number {n}"), &body, NOW + n))
        .collect();
    let (files, _) = store.load_all_unfiltered();
    assert!(files.len() >= 3, "{} files", files.len());
    for f in &files {
        let text = f.topic.render();
        assert!(text.len() <= super::format::FILE_BYTES && text.lines().count() < super::format::FILE_LINES, "{}", f.rel);
    }
    for id in &ids {
        assert!(read(&store, Audience::Companion, id, NOW).is_ok(), "{id} lost");
    }
    // The oldest notes moved to the continuation; the newest stayed.
    assert!(store.root().join("person/big-part2.md").is_file());
    assert_eq!(read_entry(&store, Audience::Companion, ids.last().unwrap()).rel, "person/big.md");
}

#[test]
fn a_summary_that_is_already_there_is_not_added_twice() {
    let (_d, store) = scratch();
    let id = rule(&store, Scope::Person, "t", "Same thing", "", NOW);
    let again = add_action(Scope::Person, "t", "same THING", Kind::Rule, Voice::Agent, "");
    assert_eq!(write(&store, Audience::Companion, NOW, &w(again)).unwrap_err(), Refusal::Duplicate(id));
}

#[test]
fn status_is_refused_with_one_fixed_reason() {
    let (_d, store) = scratch();
    for (summary, body) in [
        ("Status: the migration is done", ""),
        ("Fixed the login bug", ""),
        ("Release", "Shipped 2.3 today"),
        ("Login bug is fixed", ""),
        ("已完成登录页", ""),
        ("进度: 一半", ""),
    ] {
        let action = add_action(Scope::Person, "t", summary, Kind::Decision, Voice::Agent, body);
        let refused = write(&store, Audience::Companion, NOW, &w(action)).unwrap_err();
        assert_eq!(refused, Refusal::Status, "{summary}");
        assert_eq!(refused.to_string(), STATUS_REFUSAL);
    }
    // A rule, a decision with its why, a gotcha and a life moment are not status.
    for (summary, body) in [
        ("Never push to main", "Why: the release gate runs on main."),
        ("Chose SQLite over Postgres", "Tried Postgres, dropped: one more daemon."),
        ("Port 9898 is the old one", ""),
    ] {
        let action = add_action(Scope::Person, "t", summary, Kind::Decision, Voice::Agent, body);
        assert!(write(&store, Audience::Companion, NOW, &w(action)).is_ok(), "{summary}");
    }
    let moment = add_action(Scope::Moments, "", "Finished my first marathon", Kind::Moment, Voice::Agent, "Felt proud; concluded I can keep going.");
    assert!(write(&store, Audience::Companion, NOW, &w(moment)).is_ok());
    // An update that turns a note into a status line is refused too.
    let id = rule(&store, Scope::Person, "u", "A rule", "", NOW);
    let edit = w(Action::Update { id, summary: Some("Done: all of it".into()), body: None, kind: None });
    assert_eq!(write(&store, Audience::Companion, NOW, &edit).unwrap_err(), Refusal::Status);
}

#[test]
fn the_persons_own_words_change_only_when_they_direct_it() {
    let (_d, store) = scratch();
    let id = person(&store, "Does not eat sugar", NOW);
    let edit = Action::Update { id: id.clone(), summary: Some("Eats little sugar".into()), body: None, kind: None };
    let refused = write(&store, Audience::Companion, NOW + 1, &w(edit.clone())).unwrap_err();
    assert!(matches!(refused, Refusal::UserVoice { .. }), "{refused:?}");
    assert!(refused.to_string().contains("user_directed"));
    for action in [Action::Void { id: id.clone(), reason: "wrong".into() }, Action::NoRaise { id: id.clone(), on: true }] {
        assert!(matches!(write(&store, Audience::Companion, NOW + 1, &w(action)).unwrap_err(), Refusal::UserVoice { .. }));
    }
    let mut directed = w(edit);
    directed.user_directed = true;
    write(&store, Audience::Companion, NOW + 1, &directed).unwrap();
    let found = read_entry(&store, Audience::Companion, &id);
    assert_eq!(found.entry.summary, "Eats little sugar");
    assert_eq!(found.entry.voice, Voice::User);
}

#[test]
fn a_moment_is_a_moment_in_moments_and_nothing_else_is() {
    let (_d, store) = scratch();
    let wrong = add_action(Scope::Person, "t", "Felt sad", Kind::Moment, Voice::Agent, "");
    assert_eq!(write(&store, Audience::Companion, NOW, &w(wrong)).unwrap_err(), Refusal::KindMismatch);
    let wrong = add_action(Scope::Moments, "", "A rule", Kind::Rule, Voice::Agent, "");
    assert_eq!(write(&store, Audience::Companion, NOW, &w(wrong)).unwrap_err(), Refusal::KindMismatch);
    let ok = add_action(Scope::Moments, "ignored", "Black Friday: regretted the spend", Kind::Moment, Voice::Agent, "Felt regret; concluded: wait a day.");
    let done = write(&store, Audience::Companion, NOW, &w(ok)).unwrap();
    assert_eq!(done.rel, format!("moments/2026/{}.md", super::day_of(NOW).chars().take(7).collect::<String>()));
}

#[test]
fn a_note_cannot_become_a_moment_or_stop_being_one() {
    let (_d, store) = scratch();
    let id = rule(&store, Scope::Person, "t", "A rule", "", NOW);
    let edit = w(Action::Update { id, summary: None, body: None, kind: Some(Kind::Moment) });
    assert_eq!(write(&store, Audience::Companion, NOW, &edit).unwrap_err(), Refusal::KindMismatch);
    let moment = add_action(Scope::Moments, "", "A day", Kind::Moment, Voice::Agent, "felt fine");
    let id = write(&store, Audience::Companion, NOW, &w(moment)).unwrap().id;
    let edit = w(Action::Update { id, summary: None, body: None, kind: Some(Kind::Rule) });
    assert_eq!(write(&store, Audience::Companion, NOW, &edit).unwrap_err(), Refusal::KindMismatch);
}

/// The v1 store keeps its backups and database in the same `memory/`
/// folder: none of it is a note, listed or read.
#[test]
fn the_rest_of_the_memory_folder_is_not_ours() {
    let (_d, store) = scratch();
    rule(&store, Scope::Person, "t", "A rule", "", NOW);
    for stray in ["backups/x.md", "lancedb/x.md", "README.md", "person-old/x.md"] {
        let path = store.root().join(stray);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "---\ntopic: x\nscope: person\n---\n\n## Stray\n<!-- id: s000000001 · kind: rule · voice: agent · created: d · updated: d · source: x -->\n").unwrap();
        assert!(read(&store, Audience::Companion, stray, NOW).is_err(), "{stray} is readable");
    }
    let (seen, problems) = super::fence::seen(&store, Audience::Companion);
    assert_eq!(seen.len(), 1);
    assert!(problems.is_empty(), "{problems:?}");
    assert!(read(&store, Audience::Companion, "s000000001", NOW).is_err());
}

// ── void and no-raise ──────────────────────────────────────────────────────

#[test]
fn a_void_note_leaves_the_index_and_the_search_but_can_still_be_read() {
    let (_d, store) = scratch();
    let id = rule(&store, Scope::Person, "t", "Uses tabs everywhere", "", NOW);
    let reasonless = w(Action::Void { id: id.clone(), reason: " ".into() });
    assert_eq!(write(&store, Audience::Companion, NOW, &reasonless).unwrap_err(), Refusal::Empty("reason"));
    write(&store, Audience::Companion, NOW, &w(Action::Void { id: id.clone(), reason: "he uses spaces".into() })).unwrap();
    assert_eq!(index(&store, Audience::Companion, None, NOW).shown, 0);
    assert!(search(&store, Audience::Companion, "tabs", 10, NOW).is_empty());
    let found = read_entry(&store, Audience::Companion, &id);
    assert_eq!(found.entry.void.as_deref(), Some("he uses spaces"));
}

#[test]
fn a_no_raise_note_says_so_in_the_index() {
    let (_d, store) = scratch();
    let id = rule(&store, Scope::Area(Area::Mind), "past", "The break-up two years ago", "", NOW);
    write(&store, Audience::Companion, NOW, &w(Action::NoRaise { id, on: true })).unwrap();
    let text = index(&store, Audience::Companion, None, NOW).text;
    assert!(text.contains("don't raise"), "{text}");
}

// ── the index ──────────────────────────────────────────────────────────────

#[test]
fn the_index_runs_person_then_here_then_used_then_newest() {
    let (_d, store) = scratch();
    let near = Scope::Project(PathBuf::from("/work/app"));
    let far = Scope::Project(PathBuf::from("/work"));
    let elsewhere = Scope::Project(PathBuf::from("/other"));
    rule(&store, elsewhere.clone(), "t", "Elsewhere old", "", NOW - 10 * DAY);
    rule(&store, elsewhere.clone(), "t", "Elsewhere new", "", NOW - DAY);
    let used = rule(&store, elsewhere.clone(), "t", "Elsewhere used", "", NOW - 50 * DAY);
    let stale = rule(&store, elsewhere, "t", "Elsewhere used long ago", "", NOW - 300 * DAY);
    rule(&store, far, "t", "Far project", "", NOW - 5 * DAY);
    rule(&store, near, "t", "Near project", "", NOW - 20 * DAY);
    person(&store, "Name is Alex", NOW - 40 * DAY);
    // One read 3 days ago counts; one 200 days ago is outside the 90-day window.
    read(&store, Audience::Work, &used, NOW - 3 * DAY).unwrap();
    read(&store, Audience::Work, &stale, NOW - 200 * DAY).unwrap();
    let got = index(&store, Audience::Work, Some(Path::new("/work/app/src")), NOW);
    let order: Vec<&str> = got
        .text
        .lines()
        .skip(1)
        .map(|l| l.trim_start_matches("- ").split(" (").next().unwrap())
        .collect();
    assert_eq!(
        order,
        ["Name is Alex", "Near project", "Far project", "Elsewhere used", "Elsewhere new", "Elsewhere old", "Elsewhere used long ago"]
    );
}

#[test]
fn what_does_not_fit_is_forgotten_but_still_found() {
    let (_d, store) = scratch();
    person(&store, "Name is Alex", NOW);
    // 250 notes, each an hour older than the one before; the oldest falls out.
    let mut oldest = String::new();
    for n in 0..250 {
        let id = rule(
            &store,
            Scope::Project(PathBuf::from("/p")),
            &format!("t{}", n / 20),
            &format!("Quiet note {n:03} zebra"),
            "",
            NOW - (n as i64) * DAY / 24,
        );
        if n == 249 {
            oldest = id;
        }
    }
    let got = index(&store, Audience::Work, None, NOW);
    assert!(got.text.lines().count() <= INDEX_LINES && got.text.len() <= INDEX_BYTES);
    assert!(got.left_out > 0 && got.text.contains("more notes not shown"));
    assert!(got.text.contains("Name is Alex"), "the person never falls out");
    assert!(!got.text.contains(&oldest), "the oldest fell out");
    // Out of the index, found by search; being found brings it back.
    let hits = search(&store, Audience::Work, "zebra 249", 5, NOW);
    assert_eq!(hits[0].entry.id, oldest);
    let back = index(&store, Audience::Work, None, NOW);
    assert!(back.text.contains(&oldest), "a note that was used returns to the index");
}

#[test]
fn the_index_is_cut_at_its_bytes_too() {
    let (_d, store) = scratch();
    for n in 0..190 {
        // Long topic names make each line long, so the bytes run out first.
        let topic = format!("topic-{n}-{}", "w".repeat(250));
        rule(&store, Scope::Person, &topic, &format!("{n:03} {}", "s".repeat(70)), "", NOW - n as i64);
    }
    let got = index(&store, Audience::Work, None, NOW);
    assert!(got.text.len() <= INDEX_BYTES);
    assert!(got.left_out > 0, "{} shown", got.shown);
    assert!(got.text.lines().count() < INDEX_LINES, "bytes ran out before lines");
    assert_eq!(got.shown + got.left_out, 190);
}

#[test]
fn an_empty_store_has_an_empty_index() {
    let (_d, store) = scratch();
    assert_eq!(index(&store, Audience::Work, None, NOW).text, "");
}

// ── search ─────────────────────────────────────────────────────────────────

#[test]
fn search_finds_chinese_and_english_and_ranks_by_how_many_words() {
    let (_d, store) = scratch();
    rule(&store, Scope::Person, "饮食", "早餐不吃糖", "血糖容易高，医生建议少糖。", NOW);
    let two = rule(&store, Scope::Person, "build", "Run the release gate before publish", "gate then manifest", NOW);
    rule(&store, Scope::Person, "build", "Publish needs his yes", "", NOW - 1);
    assert_eq!(search(&store, Audience::Work, "饮食习惯 血糖", 5, NOW).len(), 1);
    assert_eq!(search(&store, Audience::Work, "糖", 5, NOW).len(), 1);
    let hits = search(&store, Audience::Work, "RELEASE publish", 5, NOW);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].entry.id, two);
    assert!(search(&store, Audience::Work, "  ", 5, NOW).is_empty());
}

// ── the fence ──────────────────────────────────────────────────────────────

#[test]
fn a_coding_reader_never_sees_body_mind_people_or_moments() {
    let (_d, store) = scratch();
    let mut hidden = Vec::new();
    for area in [Area::Body, Area::Mind, Area::People] {
        hidden.push(rule(&store, Scope::Area(area), "t", &format!("Private {} fact secretword", area.word()), "", NOW));
    }
    let moment = add_action(Scope::Moments, "", "A private moment secretword", Kind::Moment, Voice::Agent, "felt it");
    hidden.push(write(&store, Audience::Companion, NOW, &w(moment)).unwrap().id);
    let shown = [Area::Music, Area::Money, Area::Play, Area::Home]
        .map(|a| rule(&store, Scope::Area(a), "t", &format!("Fine {} fact", a.word()), "", NOW));
    let project = rule(&store, Scope::Project(PathBuf::from("/p")), "t", "Project fact", "", NOW);
    let me = person(&store, "Person fact", NOW);

    // The index, a search, a read by id, a read by path, a write by id: none reach them.
    let text = index(&store, Audience::Work, Some(Path::new("/p")), NOW).text;
    assert!(!text.contains("Private") && !text.contains("private moment"), "{text}");
    assert!(search(&store, Audience::Work, "secretword", 10, NOW).is_empty());
    for id in &hidden {
        assert_eq!(read(&store, Audience::Work, id, NOW).unwrap_err(), ReadError::NotFound);
        let edit = w(Action::Void { id: id.clone(), reason: "x".into() });
        assert_eq!(write(&store, Audience::Work, NOW, &edit).unwrap_err(), Refusal::NotFound);
    }
    let month = super::day_of(NOW).chars().take(7).collect::<String>();
    for path in ["areas/body/t.md", "areas/mind/t.md", "areas/people/t.md", &format!("moments/2026/{month}.md")] {
        assert!(matches!(read(&store, Audience::Work, path, NOW), Err(ReadError::Open(_))), "{path}");
    }
    let add = add_action(Scope::Area(Area::Mind), "t", "Sneaky", Kind::Rule, Voice::Agent, "");
    assert_eq!(write(&store, Audience::Work, NOW, &w(add)).unwrap_err(), Refusal::Fenced);
    for id in shown.iter().chain([&project, &me]) {
        assert!(read(&store, Audience::Work, id, NOW).is_ok());
    }
    // Yinyue sees all of it.
    for id in &hidden {
        assert!(read(&store, Audience::Companion, id, NOW).is_ok());
    }
    assert_eq!(search(&store, Audience::Companion, "secretword", 10, NOW).len(), 4);
}

#[test]
fn the_companion_in_a_coding_session_is_a_coding_reader() {
    assert_eq!(audience_of(true, false), Audience::Companion);
    assert_eq!(audience_of(true, true), Audience::Work);
    assert_eq!(audience_of(false, false), Audience::Work);
    assert_eq!(audience_of(false, true), Audience::Work);
}

#[test]
fn a_path_cannot_leave_the_store_or_reach_its_state() {
    let (_d, store) = scratch();
    rule(&store, Scope::Person, "t", "A rule", "", NOW);
    std::fs::write(store.root().parent().unwrap().join("outside.md"), "x").unwrap();
    for path in ["../outside.md", ".state/uses.jsonl", ".state/history/x.md", "person/../../outside.md", "/etc/hosts", "person/t.txt", "person", ""] {
        assert!(read(&store, Audience::Companion, path, NOW).is_err(), "{path}");
    }
    assert!(read(&store, Audience::Companion, "person/t.md", NOW).is_ok());
    let absolute = store.root().join("person/t.md");
    assert!(read(&store, Audience::Companion, absolute.to_str().unwrap(), NOW).is_ok());
}

// ── the tool's ops ─────────────────────────────────────────────────────────

fn caller(audience: Audience) -> Caller {
    Caller { audience, source: "claude-code".into(), cwd: Some(PathBuf::from("/work/app")) }
}

#[test]
fn the_ops_write_read_search_and_index_as_data() {
    let (_d, store) = scratch();
    let me = caller(Audience::Work);
    let added = call(
        &store,
        &me,
        "write",
        &json!({"scope": "project:/work/app", "topic": "build", "summary": "Use just test", "kind": "rule", "body": "Why: it sets the scratch home."}),
        NOW,
    )
    .unwrap();
    let id = added["id"].as_str().unwrap();
    assert_eq!(added["path"], "projects/-work-app/build.md");
    let got = call(&store, &me, "read", &json!({"id": id}), NOW).unwrap();
    assert_eq!(got["source"], "claude-code");
    assert_eq!(got["scope"], "project:/work/app");
    assert_eq!(call(&store, &me, "search", &json!({"words": "scratch"}), NOW).unwrap()["hits"].as_array().unwrap().len(), 1);
    assert!(call(&store, &me, "index", &json!({}), NOW).unwrap()["index"].as_str().unwrap().contains("Use just test"));
    // Said wrong: told what is missing, not a crash.
    assert!(call(&store, &me, "write", &json!({"summary": "x"}), NOW).is_err());
    assert!(call(&store, &me, "write", &json!({"scope": "nowhere", "summary": "x", "kind": "rule"}), NOW).is_err());
    assert!(call(&store, &me, "bogus", &json!({}), NOW).is_err());
    let status = call(&store, &me, "write", &json!({"scope": "person", "summary": "Status: done", "kind": "rule"}), NOW).unwrap_err();
    assert_eq!(status, STATUS_REFUSAL);
}

// ── the locks ──────────────────────────────────────────────────────────────

fn source_of(file: &str) -> String {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/own_memory");
    std::fs::read_to_string(dir.join(file)).unwrap()
}

/// Only the fence loads topics for a caller, and only the writer saves.
/// Another file that does either has made a second way into or out of the
/// store, and the fence or the writer's rules no longer hold.
#[test]
fn only_the_fence_loads_and_only_write_saves() {
    for file in ["call.rs", "chronicle.rs", "companion.rs", "find.rs", "format.rs", "index.rs", "migrate.rs", "mod.rs", "page.rs", "phone.rs", "write.rs"] {
        let src = source_of(file);
        assert!(!src.contains("load_all_unfiltered("), "{file} loads topics around the fence");
        assert!(!src.contains("fs::write") && !src.contains("fs::rename") && !src.contains("File::create"), "{file} writes a file itself");
    }
    for file in ["call.rs", "chronicle.rs", "companion.rs", "find.rs", "format.rs", "index.rs", "migrate.rs", "mod.rs", "fence.rs", "page.rs", "phone.rs"] {
        assert!(!source_of(file).contains(".save("), "{file} saves a topic; only write.rs does");
    }
    for file in ["find.rs", "index.rs", "call.rs", "chronicle.rs", "companion.rs", "migrate.rs", "page.rs", "phone.rs"] {
        assert!(!source_of(file).contains(".load("), "{file} reads a file around the fence");
    }
}

/// The decision of who sees what is made in the fence alone: no other file
/// names a fenced area.
#[test]
fn the_only_audience_decision_is_in_the_fence() {
    for file in ["call.rs", "chronicle.rs", "companion.rs", "find.rs", "index.rs", "page.rs", "phone.rs", "write.rs", "store.rs"] {
        let src = source_of(file);
        assert!(!src.contains("Area::Body") && !src.contains("Area::Mind"), "{file} names a fenced area");
    }
}

/// Who reads `[memory] linggen`: the engine's settings map, its prompt (and
/// the tools it offers), the turn's recall, and the chronicle's own step
/// (`ling chronicle`, which keeps the chronicle only under v2), and the
/// door the phone reaches memory by (which answers the owner's phone from
/// the notes under v2). Another
/// reader is a second place deciding which memory Linggen has.
#[test]
fn the_switch_is_read_in_the_listed_places_only() {
    fn walk(base: &Path, dir: &Path, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, found);
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if !name.ends_with(".rs") || name.contains("test") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let code: String = text
                .split("#[cfg(test)]\nmod tests {")
                .next()
                .unwrap_or_default()
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            if code.contains("memory_linggen") || code.contains(".memory.linggen") {
                found.push(path.strip_prefix(base).unwrap().display().to_string());
            }
        }
    }
    let engine = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = Vec::new();
    walk(engine, &engine.join("src"), &mut found);
    let binary = engine.join("../..").canonicalize().unwrap();
    walk(&binary, &binary.join("src"), &mut found);
    found.sort();
    assert_eq!(
        found,
        ["src/cli/chronicle.rs", "src/engine/prompt/mod.rs", "src/engine/types.rs", "src/server/api/memory.rs", "src/server/chat/runtime.rs"],
        "the switch is read in the engine's settings map, its prompt, the turn's recall, the chronicle's step and the phone's door alone"
    );
}

// ── what a session is told ─────────────────────────────────────────────────

#[test]
fn a_persons_session_is_told_how_to_use_memory_and_given_the_index() {
    let (_d, store) = scratch();
    let empty = super::prompt::owner_block(&store, Audience::Work, Path::new("/work/app"), NOW);
    assert!(empty.contains("No notes yet") && empty.contains("memory_notes op=write"), "{empty}");
    person(&store, "Name is Alex", NOW);
    rule(&store, Scope::Project(PathBuf::from("/work/app")), "build", "Use just test", "", NOW);
    let work = super::prompt::owner_block(&store, Audience::Work, Path::new("/work/app/src"), NOW);
    assert!(work.contains("Name is Alex") && work.contains("Use just test") && !work.contains("life moment"), "{work}");
    let hers = super::prompt::owner_block(&store, Audience::Companion, Path::new("/"), NOW);
    assert!(hers.contains("life moment"), "{hers}");
    // Said once, in the words the writer enforces: never what was done.
    assert!(work.contains("Never what was done"));
}

#[test]
fn an_app_is_given_the_index_alone_and_the_person_only_when_it_has_core() {
    let (_d, store) = scratch();
    person(&store, "Name is Alex", NOW);
    rule(&store, Scope::Project(PathBuf::from("/skills/dice")), "t", "Dice rule", "", NOW);
    let with = super::prompt::app_block(&store, Some(Path::new("/skills/dice")), true, NOW).unwrap();
    assert!(with.contains("Name is Alex") && with.contains("Dice rule"));
    let without = super::prompt::app_block(&store, Some(Path::new("/skills/dice")), false, NOW).unwrap();
    assert!(!without.contains("Name is Alex") && without.contains("Dice rule"));
    assert!(!without.contains("memory_notes op=write"), "an app has no tool: {without}");
    assert_eq!(super::prompt::app_block(&store, Some(Path::new("/skills/none")), false, NOW), None);
}

#[test]
fn the_switch_is_v2_unless_the_file_says_v1() {
    use super::Mode;
    assert_eq!(Mode::default(), Mode::V2);
    assert_eq!((Mode::V1.word(), Mode::V2.word()), ("v1", "v2"));
}

// ── the chronicle ──────────────────────────────────────────────────────────

use super::chronicle::{self, period_of_rel};
use super::format::yearly_day;
use crate::state_fs::sessions::{ChatMsg, SessionMeta, SessionStore};

fn chronicle_line(store: &Store, topic: &str, summary: &str, at: i64) -> Result<super::write::Written, Refusal> {
    let action = add_action(Scope::Chronicle, topic, summary, Kind::Chronicle, Voice::Agent, "Source: a note.");
    write(store, Audience::Chronicler, at, &w(action))
}

#[test]
fn the_chronicler_writes_the_chronicle_into_its_month_and_nothing_else() {
    let (_d, store) = scratch();
    // What was done is what a chronicle holds: the status rule does not apply.
    let done = chronicle_line(&store, "2026-10", "2026-10-03 Finished the move to Berlin", NOW).unwrap();
    assert_eq!(done.rel, "chronicle/2026/2026-10.md");
    let year = chronicle_line(&store, "2025", "2025 The year of the research", NOW).unwrap();
    assert_eq!(year.rel, "chronicle/2025/2025.md");
    assert_eq!(chronicle_line(&store, "october", "x", NOW).unwrap_err(), Refusal::NotAPeriod);
    assert_eq!(chronicle_line(&store, "2026-13", "x", NOW).unwrap_err(), Refusal::NotAPeriod);
    // No other scope, and no other kind in the chronicle.
    let person = add_action(Scope::Person, "t", "A fact", Kind::Person, Voice::Agent, "");
    assert_eq!(write(&store, Audience::Chronicler, NOW, &w(person)).unwrap_err(), Refusal::Fenced);
    let wrong = add_action(Scope::Chronicle, "2026-10", "A rule", Kind::Rule, Voice::Agent, "");
    assert_eq!(write(&store, Audience::Companion, NOW, &w(wrong)).unwrap_err(), Refusal::KindMismatch);
    let id = rule(&store, Scope::Person, "t", "A rule", "", NOW);
    let edit = w(Action::Update { id, summary: Some("Changed".into()), body: None, kind: None });
    assert_eq!(write(&store, Audience::Chronicler, NOW, &edit).unwrap_err(), Refusal::Fenced);
    // A coding host cannot write it, and a month split in parts is still its month.
    let sneaky = add_action(Scope::Chronicle, "2026-10", "2026-10-04 x", Kind::Chronicle, Voice::Agent, "");
    assert_eq!(write(&store, Audience::Work, NOW, &w(sneaky)).unwrap_err(), Refusal::Fenced);
    assert_eq!(period_of_rel("chronicle/2026/2026-10-part2.md").as_deref(), Some("2026-10"));
    assert_eq!(period_of_rel("chronicle/2025/2025.md").as_deref(), Some("2025"));
}

#[test]
fn only_the_companion_and_the_chronicler_see_the_chronicle() {
    let (_d, store) = scratch();
    person(&store, "Name is Alex", NOW);
    let id = chronicle_line(&store, "2026-10", "2026-10-03 Moved to Berlin", NOW).unwrap().id;
    for audience in [Audience::Companion, Audience::Chronicler] {
        assert_eq!(search(&store, audience, "Berlin", 5, NOW).len(), 1, "{audience:?}");
        assert!(read(&store, audience, "chronicle/2026/2026-10.md", NOW).is_ok());
    }
    assert!(search(&store, Audience::Work, "Berlin", 5, NOW).is_empty());
    assert!(read(&store, Audience::Work, &id, NOW).is_err());
    assert!(read(&store, Audience::Work, "chronicle/2026/2026-10.md", NOW).is_err());
    // Long, and found by search: not in anyone's index.
    for audience in [Audience::Companion, Audience::Work] {
        assert!(!index(&store, audience, None, NOW).text.contains("Berlin"));
    }
}

fn user_session(id: &str, creator: &str, skill: Option<&str>) -> SessionMeta {
    SessionMeta {
        id: id.into(),
        title: format!("title of {id}"),
        created_at: 1,
        creator: creator.into(),
        skill: skill.map(String::from),
        cwd: "/work/app".into(),
        ..Default::default()
    }
}

fn say(store: &SessionStore, id: &str, from: &str, text: &str, at: i64) {
    let row = json!({"agent_id": "ling", "from_id": from, "to_id": "ling", "content": text, "timestamp": at, "is_observation": false});
    store.add_chat_message(id, &serde_json::from_value::<ChatMsg>(row).unwrap()).unwrap();
}

fn scratch_sessions(dir: &tempfile::TempDir) -> SessionStore {
    SessionStore::with_sessions_dir(dir.path().join("sessions"))
}

#[test]
fn the_months_still_to_write_are_those_with_something_to_say_and_no_file() {
    let (dir, store) = scratch();
    let sessions = scratch_sessions(&dir);
    // July: only a session the person spoke in (a mission's, an app's and a
    // hidden shell row are not their words). August: a void and a no-raise
    // note, which say nothing. September, October: notes. November: this month.
    for (id, creator, skill) in [("s-mine", "user", None), ("s-mission", "mission", None), ("s-app", "user", Some("diary"))] {
        sessions.add_session(&user_session(id, creator, skill)).unwrap();
        say(&sessions, id, "user", "Let us plan the trip to Kyoto", NOW - 120 * DAY);
    }
    say(&sessions, "s-mine", "user", "[HIDDEN] greeting", NOW - 120 * DAY);
    say(&sessions, "s-mine", "ling", "Sure.", NOW - 120 * DAY);
    let gone = rule(&store, Scope::Person, "t", "Wrong fact", "", NOW - 90 * DAY);
    write(&store, Audience::Companion, NOW - 90 * DAY, &w(Action::Void { id: gone, reason: "no".into() })).unwrap();
    let quiet = rule(&store, Scope::Person, "t", "Private matter", "", NOW - 90 * DAY);
    write(&store, Audience::Companion, NOW - 90 * DAY, &w(Action::NoRaise { id: quiet, on: true })).unwrap();
    rule(&store, Scope::Person, "t", "Chose Rust for the engine", "Why: one binary.", NOW - 60 * DAY);
    rule(&store, Scope::Person, "t2", "Prefers just over make", "", NOW - 30 * DAY);
    rule(&store, Scope::Person, "t3", "Made this month", "", NOW);

    let got = chronicle::prepare(&store, &sessions, NOW).expect("months wait");
    assert_eq!(got.months, ["2026-07", "2026-09", "2026-10"]);
    assert!(got.years.is_empty(), "this year is not over");
    let lines: Vec<serde_json::Value> = got.text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines[0]["header"], true);
    let july = &lines[1];
    assert_eq!(july["month"], "2026-07");
    let said = july["sessions"].as_array().unwrap();
    assert_eq!(said.len(), 1, "{july}");
    assert_eq!(said[0]["title"], "title of s-mine");
    assert_eq!(said[0]["turns"], 1);
    assert!(!got.text.contains("Wrong fact") && !got.text.contains("Private matter") && !got.text.contains("Made this month"));
    assert!(got.text.contains("Chose Rust for the engine"));

    // A month with a file is not offered again, however little it says.
    chronicle_line(&store, "2026-09", "2026-09 quiet month", NOW).unwrap();
    assert_eq!(chronicle::prepare(&store, &sessions, NOW).unwrap().months, ["2026-07", "2026-10"]);
}

#[test]
fn nothing_waiting_is_none_and_a_run_is_capped_oldest_first() {
    let (dir, store) = scratch();
    let sessions = scratch_sessions(&dir);
    assert!(chronicle::prepare(&store, &sessions, NOW).is_none(), "an empty store has nothing to write");
    for back in 1..=14 {
        rule(&store, Scope::Person, "t", &format!("Note {back}"), "", NOW - back * 31 * DAY);
    }
    let got = chronicle::prepare(&store, &sessions, NOW).unwrap();
    assert_eq!(got.months.len(), chronicle::MONTHS_PER_RUN);
    assert!(got.months.windows(2).all(|m| m[0] < m[1]), "{:?}", got.months);
}

#[test]
fn a_past_year_is_summed_up_once_all_its_months_are_written() {
    let (dir, store) = scratch();
    let sessions = scratch_sessions(&dir);
    rule(&store, Scope::Person, "t", "Started the research", "", NOW - 365 * DAY);
    rule(&store, Scope::Person, "t", "Wrote up the research paper", "", NOW - 335 * DAY);
    // Two months of 2025 wait, and the year waits for them in the same run.
    let got = chronicle::prepare(&store, &sessions, NOW).unwrap();
    assert_eq!(got.months, ["2025-11", "2025-12"]);
    assert_eq!(got.years, ["2025"]);
    let year: serde_json::Value = serde_json::from_str(got.text.lines().last().unwrap()).unwrap();
    assert_eq!(year["month_files"], json!(["chronicle/2025/2025-11.md", "chronicle/2025/2025-12.md"]));
    // Months written, the year still is.
    chronicle_line(&store, "2025-11", "2025-11-24 Started the research", NOW).unwrap();
    chronicle_line(&store, "2025-12", "2025-12-24 Wrote up the research paper", NOW).unwrap();
    let after = chronicle::prepare(&store, &sessions, NOW).unwrap();
    assert!(after.months.is_empty() && after.years == ["2025"], "{after:?}");
    chronicle_line(&store, "2025", "2025 Research, from start to paper", NOW).unwrap();
    assert!(chronicle::prepare(&store, &sessions, NOW).is_none());
}

// ── Yinyue's day ───────────────────────────────────────────────────────────

use super::companion;

fn moment_at(store: &Store, summary: &str, at: i64) -> String {
    let action = add_action(Scope::Moments, "", summary, Kind::Moment, Voice::Agent, "Felt regret; concluded: wait a day.");
    write(store, Audience::Companion, at, &w(action)).unwrap().id
}

fn yearly(store: &Store, summary: &str, day: &str) -> String {
    let action = add_action(Scope::Person, "days", summary, Kind::Person, Voice::Agent, "");
    let id = write(store, Audience::Companion, NOW, &w(action)).unwrap().id;
    write(store, Audience::Companion, NOW, &w(Action::Yearly { id: id.clone(), day: Some(day.into()) })).unwrap();
    id
}

#[test]
fn the_day_has_the_same_date_in_past_years_and_the_recurring_days_near_it() {
    let (_d, store) = scratch();
    // 2026-11-25 today. Same date a year and two years ago; an other date; a chronicle line.
    moment_at(&store, "Black Friday: regretted the spend", NOW - 365 * DAY);
    moment_at(&store, "A quiet Sunday", NOW - 370 * DAY);
    chronicle_line(&store, "2024-11", "2024-11-25 Moved to Berlin", NOW).unwrap();
    chronicle_line(&store, "2024-11", "2024-11 Settled in", NOW).unwrap();
    yearly(&store, "Mom's birthday", "1966-11-27");
    yearly(&store, "Our anniversary", "11-25");
    yearly(&store, "A day far off", "03-01");
    let text = companion::today(&store, NOW);
    assert!(text.contains("1 year ago (2025-11-25) · Black Friday: regretted the spend"), "{text}");
    assert!(text.contains("2 years ago (2024-11-25) · Moved to Berlin"), "{text}");
    assert!(!text.contains("A quiet Sunday") && !text.contains("Settled in"), "{text}");
    assert!(text.contains("in 2 days · Mom's birthday (every year 11-27, 60 years"), "{text}");
    assert!(text.contains("today · Our anniversary"), "{text}");
    assert!(!text.contains("A day far off"));
    // Nothing near today: nothing said.
    assert_eq!(companion::today(&store, NOW + 100 * DAY), "");
}

#[test]
fn what_she_must_not_raise_is_never_in_her_day() {
    let (_d, store) = scratch();
    let hurt = moment_at(&store, "The break-up", NOW - 365 * DAY);
    let gone = moment_at(&store, "A wrong memory", NOW - 730 * DAY);
    let birthday = yearly(&store, "A birthday not to mention", "11-26");
    assert!(companion::today(&store, NOW).contains("The break-up"));
    write(&store, Audience::Companion, NOW, &w(Action::NoRaise { id: hurt, on: true })).unwrap();
    write(&store, Audience::Companion, NOW, &w(Action::NoRaise { id: birthday, on: true })).unwrap();
    write(&store, Audience::Companion, NOW, &w(Action::Void { id: gone, reason: "wrong".into() })).unwrap();
    let text = companion::today(&store, NOW);
    for hidden in ["The break-up", "A wrong memory", "A birthday not to mention"] {
        assert!(!text.contains(hidden), "{hidden} is in her day: {text}");
    }
    // Her index still tells her it is there, marked, so she can answer if asked.
    let block = super::prompt::owner_block(&store, Audience::Companion, Path::new("/"), NOW);
    assert!(block.contains("The break-up") && block.contains("don't raise"), "{block}");
    assert!(block.contains("never bring up yourself"), "{block}");
}

#[test]
fn only_the_companions_prompt_has_her_day_and_one_place_builds_it() {
    let (_d, store) = scratch();
    yearly(&store, "Our anniversary", "11-25");
    let hers = super::prompt::owner_block(&store, Audience::Companion, Path::new("/"), NOW);
    assert!(hers.contains("today · Our anniversary") && hers.contains("action=yearly"), "{hers}");
    for audience in [Audience::Work, Audience::Chronicler] {
        let block = super::prompt::owner_block(&store, audience, Path::new("/"), NOW);
        assert!(!block.contains("Today, ") && !block.contains("action=yearly"), "{audience:?}: {block}");
    }
    for file in ["call.rs", "chronicle.rs", "fence.rs", "find.rs", "index.rs", "write.rs"] {
        assert!(!source_of(file).contains("companion::today"), "{file} builds her day; only prompt.rs asks for it");
    }
    assert!(source_of("prompt.rs").contains("companion::today"));
}

#[test]
fn a_yearly_day_is_a_real_day_and_survives_the_file() {
    assert_eq!(yearly_day("11-26").as_deref(), Some("11-26"));
    assert_eq!(yearly_day("02-29").as_deref(), Some("02-29"));
    assert_eq!(yearly_day("1966-11-26").as_deref(), Some("1966-11-26"));
    for bad in ["", "13-01", "11-31", "1966/11/26", "tomorrow", "1966-13-01"] {
        assert_eq!(yearly_day(bad), None, "{bad}");
    }
    let (_d, store) = scratch();
    let id = yearly(&store, "Mom's birthday", "1966-11-26");
    let got = read_entry(&store, Audience::Companion, &id);
    assert_eq!(got.entry.yearly.as_deref(), Some("1966-11-26"));
    let bad = w(Action::Yearly { id: id.clone(), day: Some("someday".into()) });
    assert_eq!(write(&store, Audience::Companion, NOW, &bad).unwrap_err(), Refusal::NotADay);
    write(&store, Audience::Companion, NOW, &w(Action::Yearly { id: id.clone(), day: None })).unwrap();
    assert_eq!(read_entry(&store, Audience::Companion, &id).entry.yearly, None);
}

// ── Settings → Memory: the page's reads and its two person-only changes ───

fn as_person(action: Action) -> Write {
    Write { user_directed: true, ..w(action) }
}

/// Un-void and delete are Settings'. The tool has no word for them, so an
/// agent cannot bring back a note the person marked wrong, nor remove one.
#[test]
fn the_tool_cannot_unvoid_or_delete() {
    let (_d, store) = scratch();
    let id = rule(&store, Scope::Person, "t", "A rule", "", NOW);
    let me = Caller { audience: Audience::Companion, source: "linggen".into(), cwd: None };
    call(&store, &me, "write", &json!({"action": "void", "id": id, "reason": "wrong"}), NOW).unwrap();
    for action in ["unvoid", "delete", "remove"] {
        let refused = call(&store, &me, "write", &json!({"action": action, "id": id}), NOW).unwrap_err();
        assert!(refused.contains("unknown action"), "{action}: {refused}");
    }
    assert!(super::page::note(&store, &id).unwrap().void.is_some());
    assert!(!super::spec::DESCRIPTION.contains("unvoid") && !super::spec::DESCRIPTION.contains("delete"));
}

#[test]
fn a_voided_note_is_brought_back_by_unvoid() {
    let (_d, store) = scratch();
    let id = rule(&store, Scope::Person, "t", "A rule", "", NOW);
    write(&store, Audience::Companion, NOW, &w(Action::Void { id: id.clone(), reason: "wrong".into() })).unwrap();
    assert!(search(&store, Audience::Companion, "rule", 5, NOW).is_empty(), "a void note is not found");
    write(&store, Audience::Companion, NOW, &w(Action::Unvoid { id: id.clone() })).unwrap();
    assert_eq!(search(&store, Audience::Companion, "rule", 5, NOW).len(), 1);
}

/// A delete takes the entry out of its file, keeps the file's last version in
/// history, removes a file left empty, and a note in the person's own voice
/// needs `user_directed` like any other change.
#[test]
fn a_deleted_note_leaves_its_file_in_history() {
    let (_d, store) = scratch();
    let keep = rule(&store, Scope::Person, "t", "Keep me", "", NOW);
    let go = rule(&store, Scope::Person, "t", "Delete me", "", NOW);
    let mine = person(&store, "He said so himself", NOW);

    let refused = write(&store, Audience::Companion, NOW, &w(Action::Delete { id: mine.clone() })).unwrap_err();
    assert!(matches!(refused, Refusal::UserVoice { .. }), "{refused:?}");

    let before = read_entry(&store, Audience::Companion, &go).version;
    write(&store, Audience::Companion, NOW, &as_person(Action::Delete { id: go.clone() })).unwrap();
    assert!(super::page::note(&store, &go).is_none());
    assert!(super::page::note(&store, &keep).is_some());
    let kept = store.at_version("person/t.md", &before).expect("the file as it was");
    assert!(kept.entries.iter().any(|e| e.id == go), "history holds the deleted note");

    // The last entry of a file takes the file with it.
    write(&store, Audience::Companion, NOW, &as_person(Action::Delete { id: keep })).unwrap();
    assert!(!store.root().join("person/t.md").exists());
    assert!(super::page::note(&store, &mine).is_some(), "the other topic is untouched");
    assert_eq!(write(&store, Audience::Companion, NOW, &as_person(Action::Delete { id: go })).unwrap_err(), Refusal::NotFound);
}

/// A delete from a page that showed an old version of a file where the same
/// note changed since is a conflict, not a delete.
#[test]
fn deleting_a_note_that_changed_since_is_a_conflict() {
    let (_d, store) = scratch();
    let id = rule(&store, Scope::Person, "t", "A rule", "first", NOW);
    let saw = read_entry(&store, Audience::Companion, &id).version;
    let edit = Action::Update { id: id.clone(), summary: None, body: Some("second".into()), kind: None };
    write(&store, Audience::Companion, NOW + 1, &w(edit)).unwrap();
    let mut late = as_person(Action::Delete { id: id.clone() });
    late.base = Some(saw);
    assert!(matches!(write(&store, Audience::Companion, NOW + 2, &late), Err(Refusal::Conflict { .. })));
    assert!(super::page::note(&store, &id).is_some());
}

/// The page sees the person's whole record, and looking counts no use.
#[test]
fn the_page_sees_everything_and_counts_no_use() {
    let (_d, store) = scratch();
    person(&store, "Name is Alex", NOW);
    let mind = add_action(Scope::Area(Area::Mind), "past", "A hard year", Kind::Person, Voice::Agent, "Felt low.");
    let mind = write(&store, Audience::Companion, NOW, &w(mind)).unwrap().id;
    let overview = super::page::overview(&store);
    assert_eq!(overview.topics.iter().map(|t| (t.path.as_str(), t.count)).collect::<Vec<_>>(), [("areas/mind/past.md", 1), ("person/about.md", 1)]);
    assert_eq!(super::page::notes_of(&store, "areas/mind/past.md").unwrap()[0].id, mind);
    assert_eq!(super::page::note(&store, &mind).unwrap().body, "Felt low.");
    assert!(super::page::notes_of(&store, "../outside.md").is_err());
    assert!(!store.root().join(".state/uses.jsonl").exists(), "the page counted a use");
}

/// Review: notes marked wrong, and notes neither read nor changed in 90
/// days. A fresh note, a read one and a changed one are not in it.
#[test]
fn the_review_lists_the_void_and_the_unused_for_ninety_days() {
    let (_d, store) = scratch();
    let long_ago = NOW - 120 * DAY;
    let old_unused = rule(&store, Scope::Person, "t", "Old and unused", "", long_ago);
    let old_read = rule(&store, Scope::Person, "t", "Old but read", "", long_ago);
    let old_edited = rule(&store, Scope::Person, "t", "Old but edited", "", long_ago);
    let fresh = rule(&store, Scope::Person, "t", "Fresh", "", NOW - 2 * DAY);
    let wrong = rule(&store, Scope::Person, "t", "Wrong one", "", NOW - 2 * DAY);
    write(&store, Audience::Companion, NOW - DAY, &w(Action::Void { id: wrong.clone(), reason: "no longer true".into() })).unwrap();
    write(&store, Audience::Companion, NOW - 10 * DAY, &w(Action::Update { id: old_edited.clone(), summary: None, body: Some("changed".into()), kind: None })).unwrap();
    read(&store, Audience::Companion, &old_read, NOW - 5 * DAY).unwrap();

    let rows = super::page::review(&store, NOW);
    let got: Vec<(&str, super::page::Why)> = rows.iter().map(|r| (r.note.id.as_str(), r.why)).collect();
    assert_eq!(got, [(wrong.as_str(), super::page::Why::Void), (old_unused.as_str(), super::page::Why::Unused)]);
    assert!(![&fresh, &old_read, &old_edited].iter().any(|id| got.iter().any(|(g, _)| g == &id.as_str())));
}

/// The chronicle is never read into an index, so "not read" says nothing of
/// it: its old lines are not in the review list. A line marked wrong still is.
#[test]
fn the_review_leaves_the_old_chronicle_alone_but_lists_a_wrong_line() {
    let (_d, store) = scratch();
    let long_ago = NOW - 200 * DAY;
    let old = chronicle_line(&store, "2026-04", "2026-04-03 Moved to Berlin", long_ago).unwrap().id;
    let wrong = chronicle_line(&store, "2026-04", "2026-04-09 A line that is wrong", long_ago).unwrap().id;
    write(&store, Audience::Companion, NOW, &w(Action::Void { id: wrong.clone(), reason: "no".into() })).unwrap();
    let rows = super::page::review(&store, NOW);
    let got: Vec<&str> = rows.iter().map(|r| r.note.id.as_str()).collect();
    assert_eq!(got, [wrong.as_str()]);
    assert!(!got.contains(&old.as_str()));
}

// ── the owner's phone ──────────────────────────────────────────────────────

mod phone_verbs {
    use super::*;
    use crate::own_memory::phone;
    use serde_json::{json, Value};

    fn ask(store: &Store, verb: &str, args: Value) -> Value {
        phone::answer(store, verb, &args, NOW).expect("the phone's verb is answered")
    }

    fn add(store: &Store, content: &str, app: &str) -> String {
        let got = ask(store, "add", json!({"content": content, "episodic": true, "host": "linggen-mobile", "cwd": format!("~/.linggen/skills/{app}")}));
        got["id"].as_str().unwrap().to_string()
    }

    fn contents(rows: &Value) -> Vec<String> {
        rows.as_array().unwrap().iter().map(|r| r["content"].as_str().unwrap().to_string()).collect()
    }

    /// A note comes back, from `list` as the phone's pull asks and from a
    /// deliberate search, as the text that was sent (a long one too), under
    /// the app's folder; the phone's pull keeps the row because its id is listed.
    #[test]
    fn a_note_added_comes_back_as_sent_with_its_id_in_the_pull() {
        let (_d, store) = scratch();
        let long = format!("Likes the window seat on trains, and {}", "always says so. ".repeat(12));
        let (short, dj) = (add(&store, "Prefers oat milk", "yinyue"), add(&store, &long, "dj"));
        let kept = ask(&store, "list", json!({"tier": "semantic", "apps": ["yinyue", "dj"], "type": "fact", "limit": 1000}));
        let ids: Vec<&str> = kept.as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&short.as_str()) && ids.contains(&dj.as_str()), "{kept}");
        let texts = contents(&kept);
        assert!(texts.contains(&"Prefers oat milk".to_string()) && texts.contains(&long.trim().to_string()), "{texts:?}");
        let row = kept.as_array().unwrap().iter().find(|r| r["id"] == dj.as_str()).unwrap();
        assert_eq!((row["tier"].as_str(), row["scope"].as_str()), (Some("semantic"), Some("~/.linggen/skills/dj")));
        let found = ask(&store, "search", json!({"query": "window seat", "limit": 5}));
        assert_eq!(contents(&found), vec![long.trim().to_string()]);
        assert_eq!(ask(&store, "list", json!({"episodic": true, "limit": 1000})), json!([]), "nothing is staged");
        let file = std::fs::read_to_string(store.root().join("areas/mind/from-the-phone.md")).unwrap();
        assert!(file.contains("source: phone") && file.contains("Prefers oat milk"), "{file}");
    }

    /// The person's own notes are the phone's core ("About them"), without a scope.
    #[test]
    fn the_persons_notes_are_the_phones_core() {
        let (_d, store) = scratch();
        let id = person(&store, "Name is Alex", NOW);
        let core = ask(&store, "list", json!({"tier": "core", "limit": 200}));
        assert_eq!(core, json!([{"id": id, "content": "Name is Alex", "tier": "core"}]));
        assert_eq!(ask(&store, "list", json!({"tier": "semantic", "type": "preference", "limit": 1000})), json!([]));
    }

    /// A note with no place named is in a place a coding session never sees:
    /// nothing of it in Ling's index, search or read; the companion has it.
    #[test]
    fn a_phone_note_with_no_place_is_never_seen_by_a_coding_host() {
        let (_d, store) = scratch();
        let id = add(&store, "Worried about the interview", "no-such-app");
        assert!(!index(&store, Audience::Work, None, NOW).text.contains("interview"));
        assert!(search(&store, Audience::Work, "interview", 5, NOW).is_empty());
        assert!(read(&store, Audience::Work, &id, NOW).is_err());
        assert!(index(&store, Audience::Companion, None, NOW).text.contains("interview"));
    }

    /// What can never succeed is answered ok without an id (the phone's queue
    /// would otherwise stop on it for good); a note said twice is the same note.
    #[test]
    fn a_note_that_cannot_land_does_not_jam_the_phone() {
        let (_d, store) = scratch();
        let status = ask(&store, "add", json!({"content": "Status: the migration is done", "cwd": "~/.linggen/skills/yinyue"}));
        assert_eq!(status["id"], "");
        assert_eq!(ask(&store, "add", json!({"content": "  "}))["id"], "");
        let first = add(&store, "Prefers oat milk", "yinyue");
        assert_eq!(add(&store, "Prefers oat milk", "yinyue"), first);
    }

    /// The person's delete from the phone removes the note, even one in their
    /// own voice; an id of the old store is not an error.
    #[test]
    fn a_delete_from_the_phone_removes_the_note_and_an_old_id_is_not_an_error() {
        let (_d, store) = scratch();
        let mine = person(&store, "Name is Alex", NOW);
        let phone_note = add(&store, "Prefers oat milk", "yinyue");
        assert_eq!(ask(&store, "delete", json!({"id": mine}))["deleted"], true);
        assert_eq!(ask(&store, "delete", json!({"id": phone_note}))["deleted"], true);
        assert_eq!(ask(&store, "delete", json!({"id": "Mw13jg0tlu"}))["deleted"], false);
        assert_eq!(ask(&store, "list", json!({"tier": "semantic"})), json!([]));
        assert_eq!(ask(&store, "list", json!({"tier": "core"})), json!([]));
    }

    /// A per-turn recall (it carries `min_score`) finds nothing — v2 has none;
    /// a note marked wrong is neither listed nor found.
    #[test]
    fn no_per_turn_recall_and_a_void_note_is_not_listed() {
        let (_d, store) = scratch();
        let id = add(&store, "Prefers oat milk", "yinyue");
        let recall = ask(&store, "search", json!({"query": "what milk should I buy", "limit": 1, "min_score": 0.8}));
        assert_eq!(recall, json!([]));
        write(&store, Audience::Companion, NOW, &w(Action::Void { id, reason: "wrong".into() })).unwrap();
        assert_eq!(ask(&store, "list", json!({"tier": "semantic"})), json!([]));
        assert_eq!(ask(&store, "search", json!({"query": "oat milk"})), json!([]));
    }

    /// Only the phone's four verbs are answered here.
    #[test]
    fn only_the_four_verbs_are_the_notes() {
        for verb in ["add", "delete", "search", "list"] {
            assert!(phone::serves(verb));
        }
        for verb in ["update", "get", "count", "restamp", "forget"] {
            assert!(!phone::serves(verb));
        }
    }
}

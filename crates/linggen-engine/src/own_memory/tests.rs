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
    assert_eq!(write(&store, Audience::Companion, NOW, &w(wrong)).unwrap_err(), Refusal::MomentMismatch);
    let wrong = add_action(Scope::Moments, "", "A rule", Kind::Rule, Voice::Agent, "");
    assert_eq!(write(&store, Audience::Companion, NOW, &w(wrong)).unwrap_err(), Refusal::MomentMismatch);
    let ok = add_action(Scope::Moments, "ignored", "Black Friday: regretted the spend", Kind::Moment, Voice::Agent, "Felt regret; concluded: wait a day.");
    let done = write(&store, Audience::Companion, NOW, &w(ok)).unwrap();
    assert_eq!(done.rel, format!("moments/2026/{}.md", super::day_of(NOW).chars().take(7).collect::<String>()));
}

#[test]
fn a_note_cannot_become_a_moment_or_stop_being_one() {
    let (_d, store) = scratch();
    let id = rule(&store, Scope::Person, "t", "A rule", "", NOW);
    let edit = w(Action::Update { id, summary: None, body: None, kind: Some(Kind::Moment) });
    assert_eq!(write(&store, Audience::Companion, NOW, &edit).unwrap_err(), Refusal::MomentMismatch);
    let moment = add_action(Scope::Moments, "", "A day", Kind::Moment, Voice::Agent, "felt fine");
    let id = write(&store, Audience::Companion, NOW, &w(moment)).unwrap().id;
    let edit = w(Action::Update { id, summary: None, body: None, kind: Some(Kind::Rule) });
    assert_eq!(write(&store, Audience::Companion, NOW, &edit).unwrap_err(), Refusal::MomentMismatch);
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
    for file in ["call.rs", "find.rs", "format.rs", "index.rs", "mod.rs", "page.rs", "write.rs"] {
        let src = source_of(file);
        assert!(!src.contains("load_all_unfiltered("), "{file} loads topics around the fence");
        assert!(!src.contains("fs::write") && !src.contains("fs::rename") && !src.contains("File::create"), "{file} writes a file itself");
    }
    for file in ["call.rs", "find.rs", "format.rs", "index.rs", "mod.rs", "fence.rs", "page.rs"] {
        assert!(!source_of(file).contains(".save("), "{file} saves a topic; only write.rs does");
    }
    for file in ["find.rs", "index.rs", "call.rs", "page.rs"] {
        assert!(!source_of(file).contains(".load("), "{file} reads a file around the fence");
    }
}

/// The decision of who sees what is made in the fence alone: no other file
/// names a fenced area.
#[test]
fn the_only_audience_decision_is_in_the_fence() {
    for file in ["call.rs", "find.rs", "index.rs", "page.rs", "write.rs", "store.rs"] {
        let src = source_of(file);
        assert!(!src.contains("Area::Body") && !src.contains("Area::Mind"), "{file} names a fenced area");
    }
}

/// Who reads `[memory] linggen`: the engine's settings map, its prompt (and
/// the tools it offers), and the turn's recall. A fourth reader is a second
/// place deciding which memory a session has.
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
        ["src/engine/prompt/mod.rs", "src/engine/types.rs", "src/server/chat/runtime.rs"],
        "the switch is read in the engine's settings map, its prompt and the turn's recall alone"
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
fn the_switch_is_v1_unless_the_file_says_v2() {
    use super::Mode;
    assert_eq!(Mode::default(), Mode::V1);
    assert_eq!((Mode::V1.word(), Mode::V2.word()), ("v1", "v2"));
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

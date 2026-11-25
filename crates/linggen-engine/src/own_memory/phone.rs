//! The owner's phone, under Memory V2: the four verbs Yinyue on the phone
//! speaks (`add`, `delete`, `search`, `list`: the old store's words, over
//! `/api/memory/<verb>`) answered from these notes. The phone is unchanged;
//! the door (`server::api::memory`) only chooses this when `[memory] linggen`
//! is v2 and the caller is the owner's own phone.
//!
//! One writer, one fence: a phone note is a [`write::write`] with source
//! `phone`, and every read is the companion's ([`fence::audience_of`]) —
//! Yinyue's, whole record. What decides what is new here is only the
//! translation: where a note goes ([`PLACES`]), and how a note reads back as
//! a row of the shape the phone keeps (`id`, `content`, `tier`, `scope`).
//!
//! The phone's queue stops at the first answer that is not ok and tries it
//! again, so a request that can never succeed (a status line, an empty note,
//! a note already gone) is answered ok, with no id: the note stays on the
//! phone, nothing jams. Only a failure worth retrying (the disk, a note
//! changed under the write) is an error.

use super::fence::{self, audience_of, Audience, Seen};
use super::find;
use super::format::{one_line, Kind, Scope, Voice, SUMMARY_CHARS};
use super::write::{write, Action, Refusal, Write};
use super::Store;
use serde_json::{json, Value};

/// The verbs answered here; any other the phone sends goes the old way.
pub fn serves(verb: &str) -> bool {
    matches!(verb, "add" | "delete" | "search" | "list")
}

/// Who writes a phone note, as the notes record it.
const SOURCE: &str = "phone";
/// The topic every phone note is filed under.
const TOPIC: &str = "From the phone";

/// Where the notes of each of the phone's apps go (`MemoryApps.all` in the
/// phone). The first is Yinyue's own and the default for an app not named:
/// a fenced place, so a note whose subject Yinyue could not say never
/// reaches a coding host (`tests::a_phone_note_with_no_place_is_never_seen_by_a_coding_host`).
/// The one table; [`app_of`] reads it back.
const PLACES: [(&str, &str); 6] = [
    ("yinyue", "area:mind"),
    ("dj", "area:music"),
    ("cfo", "area:money"),
    ("health", "area:body"),
    ("shifu", "area:home"),
    ("photos", "area:play"),
];

/// Where the phone keeps an app's rows on the Mac: the skill's folder.
const SKILLS: &str = "~/.linggen/skills";

fn audience() -> Audience {
    audience_of(true, false)
}

/// One verb's answer (the `data` of the old store's envelope), or why it
/// should be tried again.
pub fn answer(store: &Store, verb: &str, args: &Value, now: i64) -> Result<Value, String> {
    match verb {
        "add" => add(store, args, now),
        "delete" => delete(store, args, now),
        "search" => Ok(search(store, args, now)),
        "list" => Ok(list(store, args)),
        other => Err(format!("the phone's notes have no verb `{other}`")),
    }
}

fn text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty())
}

/// The app a row was filed for: the one named in `cwd`
/// (`~/.linggen/skills/<app>`); Yinyue's when none.
fn app_named(args: &Value) -> &str {
    let cwd = text(args, "cwd").unwrap_or_default();
    let app = cwd.split_once("/skills/").map_or("", |(_, rest)| rest.split('/').next().unwrap_or(""));
    PLACES.iter().map(|(name, _)| *name).find(|name| *name == app).unwrap_or(PLACES[0].0)
}

/// The app the notes of `scope` serve; Yinyue's for a place no app has.
fn app_of(scope: &str) -> &'static str {
    PLACES.iter().find(|(_, place)| *place == scope).map_or(PLACES[0].0, |(app, _)| *app)
}

fn place_of(app: &str) -> Scope {
    let (_, place) = PLACES.iter().find(|(name, _)| *name == app).unwrap_or(&PLACES[0]);
    Scope::parse(place).expect("a place in PLACES is a scope")
}

fn add(store: &Store, args: &Value, now: i64) -> Result<Value, String> {
    let content = text(args, "content").unwrap_or_default();
    let summary = one_line(content, SUMMARY_CHARS);
    // The body carries the text when the summary has cut it short.
    let body = if one_line(content, usize::MAX) == summary { "" } else { content };
    let action = Action::Add {
        scope: place_of(app_named(args)),
        topic: TOPIC.to_string(),
        summary,
        kind: Kind::Person,
        voice: Voice::Agent,
        body: body.to_string(),
    };
    match write(store, audience(), now, &phone_write(action)) {
        Ok(done) => Ok(json!({ "id": done.id })),
        Err(Refusal::Duplicate(id)) => Ok(json!({ "id": id })),
        Err(retry) if retriable(&retry) => Err(retry.to_string()),
        Err(never) => {
            tracing::info!("phone note not kept in Memory V2: {}", never.code());
            Ok(json!({ "id": "", "refused": never.code() }))
        }
    }
}

/// The person deleted a row on the phone: the notes' own delete, said by
/// the person (`user_directed`), so a note in their voice goes too.
fn delete(store: &Store, args: &Value, now: i64) -> Result<Value, String> {
    let Some(id) = text(args, "id") else {
        return Ok(json!({ "deleted": false }));
    };
    let w = Write { user_directed: true, ..phone_write(Action::Delete { id: id.to_string() }) };
    match write(store, audience(), now, &w) {
        Ok(done) => Ok(json!({ "id": done.id, "deleted": true })),
        // An id of the old store, or one already gone: nothing to delete.
        Err(Refusal::NotFound) => Ok(json!({ "deleted": false })),
        Err(retry) if retriable(&retry) => Err(retry.to_string()),
        Err(never) => {
            tracing::info!("phone delete not done in Memory V2: {}", never.code());
            Ok(json!({ "deleted": false, "refused": never.code() }))
        }
    }
}

fn phone_write(action: Action) -> Write {
    Write { action, user_directed: false, source: SOURCE.to_string(), base: None }
}

/// Worth trying again: the disk, or the note changed under the write.
fn retriable(refusal: &Refusal) -> bool {
    matches!(refusal, Refusal::Disk(_) | Refusal::Conflict { .. } | Refusal::ReadAgain)
}

/// What a search finds, as rows. A search that carries `min_score` is the
/// phone's per-turn recall: Memory V2 recalls nothing per turn (what Yinyue
/// knows is in her prompt, from `list`), so it finds nothing.
fn search(store: &Store, args: &Value, now: i64) -> Value {
    if args.get("min_score").is_some() {
        return json!([]);
    }
    let Some(words) = text(args, "query") else {
        return json!([]);
    };
    let limit = args.get("limit").and_then(Value::as_u64).map_or(5, |n| (n as usize).clamp(1, 30));
    Value::Array(find::search(store, audience(), words, limit, now).iter().map(|f| row(&f.scope, &f.entry)).collect())
}

/// What the phone keeps a copy of: `core` is the person's notes (who they
/// are), `semantic` the notes of the places the phone's apps have. Nothing is
/// staged (there is no dream to wait on), and moments, the chronicle and
/// work notes are not the phone's.
fn list(store: &Store, args: &Value) -> Value {
    let limit = args.get("limit").and_then(Value::as_u64).map_or(1000, |n| n as usize);
    let wants = |s: &Seen| match (args.get("episodic").and_then(Value::as_bool), args.get("tier").and_then(Value::as_str)) {
        (Some(true), _) => false,
        (_, Some("core")) => s.scope == Scope::Person,
        (_, Some("semantic")) => matches!(s.scope, Scope::Area(_)),
        _ => false,
    };
    let mut seen: Vec<Seen> = fence::seen(store, audience()).0.into_iter().filter(|s| s.entry.void.is_none() && wants(s)).collect();
    seen.sort_by(|a, b| b.entry.updated.cmp(&a.entry.updated).then(a.entry.id.cmp(&b.entry.id)));
    seen.truncate(limit);
    Value::Array(seen.iter().map(|s| row(&s.scope.word(), &s.entry)).collect())
}

/// A note as the phone keeps a row: its text, its tier and the app's folder
/// as its scope (the person's own notes have none, as a core row never did).
fn row(scope: &str, e: &super::format::Entry) -> Value {
    let person = scope == "person";
    let mut row = json!({ "id": e.id, "content": content_of(e), "tier": if person { "core" } else { "semantic" } });
    if !person {
        row["scope"] = json!(format!("{SKILLS}/{}", app_of(scope)));
    }
    row
}

/// A note's text as one line of the phone's: the summary, and the body when
/// it says more (a body that begins with the summary is the whole text).
fn content_of(e: &super::format::Entry) -> String {
    match e.body.trim() {
        "" => e.summary.clone(),
        body if one_line(body, SUMMARY_CHARS) == e.summary => body.to_string(),
        body => format!("{} — {body}", e.summary),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_place_is_a_scope_and_the_default_is_the_first() {
        for (app, place) in PLACES {
            assert!(Scope::parse(place).is_some(), "{app}: {place}");
        }
        assert_eq!(PLACES[0].0, "yinyue");
    }
}

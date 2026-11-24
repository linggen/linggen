//! Settings → Memory sources: what Linggen remembers, by source, for its
//! owner to see and tend.
//!
//!   GET    /api/remembered-sources                           the sources, each with a count
//!   GET    /api/remembered-sources/{source}/topics           a source's topics
//!   GET    /api/remembered-sources/{source}/entries?topic=   a topic's entries
//!   GET    /api/remembered-sources/{source}/note?id=|path=   one entry's text
//!   GET    /api/remembered-notes/review                      Linggen's notes to look at again
//!   PUT    /api/remembered-notes/{id}                        edit a note's summary / body / kind
//!   POST   /api/remembered-notes/{id}/void | unvoid | no-raise
//!   DELETE /api/remembered-notes/{id}                        take it out (the file's last version stays in history)
//!
//! `{source}` is `linggen` (Linggen's own notes, `own_memory`) or an agent's
//! id (`outside_memory`: Claude Code, Codex, whatever the registry lists).
//! Only Linggen's own notes can be changed: the routes that change a note
//! carry no source and know the one store (`own_memory::write`, the one
//! writer); another agent's memory is read through `outside_memory`, which
//! has no write. Every route is admin (`routes/classes.rs`). An edit here is
//! the person's own, so it is written `user_directed`.

use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::outside_memory::{self as outside, Env};
use crate::own_memory::format::Kind;
use crate::own_memory::write::{write, Action, Refusal, Write};
use crate::own_memory::{page, Store};

/// Linggen's own notes, as a source.
const OWN: &str = "linggen";
/// What an edit made here is signed as.
const SIGNED: &str = "linggen";

fn fail(status: StatusCode, code: &str, detail: &str) -> Response {
    (status, Json(json!({ "error": code, "detail": detail }))).into_response()
}

fn refused(r: Refusal) -> Response {
    let status = match r {
        Refusal::NotFound => StatusCode::NOT_FOUND,
        Refusal::Conflict { .. } | Refusal::ReadAgain => StatusCode::CONFLICT,
        Refusal::Disk(_) => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::UNPROCESSABLE_ENTITY,
    };
    fail(status, r.code(), &r.to_string())
}

fn no_home() -> Response {
    fail(StatusCode::INTERNAL_SERVER_ERROR, "no_home", "this machine has no home folder")
}

fn machine() -> Option<(Store, Env)> {
    Some((Store::this_machine()?, Env::this_machine()?))
}

fn now() -> i64 {
    crate::util::now_ts_secs() as i64
}

/// The tilde form a page shows and sends back.
fn shown(env: &Env, path: &std::path::Path) -> String {
    env.tilde(path)
}

/// The last two parts of a path: `-Users-x-app/memory`.
fn tail(path: &std::path::Path) -> String {
    let parts: Vec<_> = path.components().filter_map(|c| c.as_os_str().to_str()).collect();
    parts[parts.len().saturating_sub(2)..].join("/")
}

/// GET /api/remembered-sources
pub(crate) async fn sources() -> Response {
    let Some((store, env)) = machine() else {
        return no_home();
    };
    let own = page::overview(&store);
    let review = page::review(&store, now()).len();
    let mut out = vec![json!({
        "id": OWN, "label": "Linggen", "own": true,
        "count": own.topics.iter().map(|t| t.count).sum::<usize>(),
        "topics": own.topics.len(),
        "voided": own.topics.iter().map(|t| t.voided).sum::<usize>(),
        "review": review,
        "unreadable": own.unreadable,
    })];
    for memory in outside::listing(&env) {
        out.push(json!({
            "id": memory.agent.id(), "label": memory.agent.label(), "own": false,
            "count": memory.places.iter().map(|p| p.files.len()).sum::<usize>(),
            "topics": memory.places.len(),
        }));
    }
    Json(json!({ "sources": out })).into_response()
}

/// GET /api/remembered-sources/{source}/topics
pub(crate) async fn topics(Path(source): Path<String>) -> Response {
    let Some((store, env)) = machine() else {
        return no_home();
    };
    if source == OWN {
        return Json(json!({ "topics": page::overview(&store).topics })).into_response();
    }
    let Some(memory) = outside::listing(&env).into_iter().find(|m| m.agent.id() == source) else {
        return fail(StatusCode::NOT_FOUND, "not_found", "no such source");
    };
    let topics: Vec<Value> = memory
        .places
        .iter()
        .map(|p| {
            json!({
                "path": shown(&env, &p.root), "name": tail(&p.root), "count": p.files.len(),
                "changed": p.files.iter().map(|f| f.changed).max().unwrap_or(0),
            })
        })
        .collect();
    Json(json!({ "topics": topics })).into_response()
}

/// GET /api/remembered-sources/{source}/entries?topic=
pub(crate) async fn entries(Path(source): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let Some((store, env)) = machine() else {
        return no_home();
    };
    let Some(topic) = q.get("topic") else {
        return fail(StatusCode::BAD_REQUEST, "bad_request", "name the topic");
    };
    if source == OWN {
        return match page::notes_of(&store, topic) {
            Ok(notes) => Json(json!({ "entries": notes })).into_response(),
            Err(e) => fail(StatusCode::NOT_FOUND, "not_found", &e.to_string()),
        };
    }
    let place = outside::listing(&env)
        .into_iter()
        .filter(|m| m.agent.id() == source)
        .flat_map(|m| m.places)
        .find(|p| shown(&env, &p.root) == *topic);
    let Some(place) = place else {
        return fail(StatusCode::NOT_FOUND, "not_found", "no such topic");
    };
    let files: Vec<Value> = place
        .files
        .iter()
        .map(|f| json!({ "path": shown(&env, &f.path), "name": f.name, "bytes": f.bytes, "changed": f.changed }))
        .collect();
    Json(json!({ "entries": files })).into_response()
}

/// GET /api/remembered-sources/{source}/note?id= (Linggen) | ?path= (another agent)
pub(crate) async fn note(Path(source): Path<String>, Query(q): Query<HashMap<String, String>>) -> Response {
    let Some((store, env)) = machine() else {
        return no_home();
    };
    if source == OWN {
        let found = q.get("id").and_then(|id| page::note(&store, id));
        return match found {
            Some(note) => Json(json!(note)).into_response(),
            None => fail(StatusCode::NOT_FOUND, "not_found", "no note with that id"),
        };
    }
    let Some(path) = q.get("path") else {
        return fail(StatusCode::BAD_REQUEST, "bad_request", "name the path");
    };
    match outside::read(&env, path) {
        Ok(n) => Json(json!({ "path": shown(&env, &n.path), "text": n.text.as_str(), "truncated": n.truncated })).into_response(),
        Err(e) => fail(StatusCode::NOT_FOUND, "not_found", &e.to_string()),
    }
}

/// GET /api/remembered-notes/review
pub(crate) async fn review() -> Response {
    let Some((store, _)) = machine() else {
        return no_home();
    };
    Json(json!({ "review": page::review(&store, now()) })).into_response()
}

#[derive(Deserialize)]
pub(crate) struct Edit {
    summary: Option<String>,
    body: Option<String>,
    kind: Option<String>,
    /// The version of the note's file the page showed.
    base: Option<String>,
}

#[derive(Deserialize, Default)]
pub(crate) struct Mark {
    reason: Option<String>,
    on: Option<bool>,
    base: Option<String>,
}

/// One change of one note, by the one writer, as the person.
fn change(id: String, base: Option<String>, action: impl FnOnce(String) -> Action) -> Response {
    let Some((store, _)) = machine() else {
        return no_home();
    };
    let w = Write { action: action(id), user_directed: true, source: SIGNED.into(), base };
    match write(&store, page::OWNER, now(), &w) {
        Ok(done) => match page::note(&store, &done.id) {
            Some(note) => Json(json!({ "note": note })).into_response(),
            None => Json(json!({ "deleted": done.id })).into_response(),
        },
        Err(r) => refused(r),
    }
}

/// PUT /api/remembered-notes/{id}
pub(crate) async fn edit(Path(id): Path<String>, Json(body): Json<Edit>) -> Response {
    let kind = match body.kind.as_deref().map(|k| (k, Kind::parse(k))) {
        Some((_, Some(k))) => Some(k),
        Some((k, None)) => return fail(StatusCode::BAD_REQUEST, "bad_request", &format!("no kind `{k}`")),
        None => None,
    };
    change(id, body.base, |id| Action::Update { id, summary: body.summary, body: body.body, kind })
}

/// POST /api/remembered-notes/{id}/void — `{ "reason": "…" }`
pub(crate) async fn void(Path(id): Path<String>, Json(body): Json<Mark>) -> Response {
    let reason = body.reason.unwrap_or_default();
    change(id, body.base, |id| Action::Void { id, reason })
}

/// POST /api/remembered-notes/{id}/unvoid — `{}`
pub(crate) async fn unvoid(Path(id): Path<String>, Json(body): Json<Mark>) -> Response {
    change(id, body.base, |id| Action::Unvoid { id })
}

/// POST /api/remembered-notes/{id}/no-raise — `{ "on": true | false }`
pub(crate) async fn no_raise(Path(id): Path<String>, Json(body): Json<Mark>) -> Response {
    let on = body.on.unwrap_or(true);
    change(id, body.base, |id| Action::NoRaise { id, on })
}

/// DELETE /api/remembered-notes/{id}
pub(crate) async fn delete(Path(id): Path<String>) -> Response {
    change(id, None, |id| Action::Delete { id })
}

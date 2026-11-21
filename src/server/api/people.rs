//! The household, as this Mac's owner sees and sets it (`server::people`):
//!
//!   GET /api/people              the owner's side, and each person with their settings and devices
//!   PUT /api/people/{person}     set what the body names of one person's settings
//!   PUT /api/people/owner        set the apps the owner keeps to themselves
//!
//! Admin class; a setting is made only by someone at this Mac
//! (`ling_mem_connect::is_at_this_mac`), never from a paired device or another
//! machine. The person is the string a person is kept as: an account's id, or
//! a signed-out phone's stamp (`GET` gives each one's).

use axum::{
    extract::{ConnectInfo, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

use crate::server::person::{of_row, Person};

fn err(code: StatusCode, msg: &str) -> Response {
    (code, Json(json!({ "error": msg }))).into_response()
}

/// GET /api/people — each person of the household who is not the owner, with
/// their settings, what they used today, and the devices that are theirs.
pub(crate) async fn list(
    State(state): State<std::sync::Arc<crate::server::ServerState>>,
) -> Response {
    let people = crate::server::people::current();
    let mut persons: Vec<(
        Person,
        Vec<Value>,
        Option<crate::state_fs::devices::AccountRef>,
    )> = Vec::new();
    for device in crate::state_fs::devices::load_devices() {
        let person = of_row(&device);
        if !person.is_another() {
            continue;
        }
        let row = json!({ "id": device.id, "name": device.name, "kind": device.kind() });
        match persons.iter_mut().find(|(p, _, _)| *p == person) {
            Some((_, rows, _)) => rows.push(row),
            None => persons.push((person, vec![row], device.account.clone())),
        }
    }
    let mut out = Vec::new();
    for (person, devices, account) in persons {
        let used = state.token_usage.lock().await.used_by(&person.wire());
        out.push(json!({
            "id": person.wire(),
            "name": crate::state_fs::devices::person_label(&account),
            "settings": people.settings_of(&person),
            "level": people.level_of(&person),
            "used_today": used,
            "devices": devices,
        }));
    }
    Json(json!({ "owner": people.owner, "people": out })).into_response()
}

/// PUT /api/people/{person} — set what the body names (`level`, `apps`,
/// `models`, `daily_tokens`, `arrival`); a bad body changes nothing.
pub(crate) async fn set_person(
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    if !crate::server::api::ling_mem_connect::is_at_this_mac(peer, &headers) {
        return err(StatusCode::FORBIDDEN, "only at this Mac");
    }
    let Some(patch) = body.as_object() else {
        return err(StatusCode::BAD_REQUEST, "a JSON object of settings");
    };
    let person = Person::from_wire(Some(&id));
    match crate::server::people::modify(|people| people.set(&person, patch)) {
        Ok(Ok(())) => {}
        Ok(Err(why)) => return err(StatusCode::BAD_REQUEST, why),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &format!("persist: {e}")),
    }
    tracing::info!("[people] settings of {id} changed");
    Json(json!({ "status": "ok", "settings": crate::server::people::current().settings_of(&person) })).into_response()
}

/// PUT /api/people/owner — `{"mine_only": [apps]}`: the apps no one else uses.
pub(crate) async fn set_owner_side(
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !crate::server::api::ling_mem_connect::is_at_this_mac(peer, &headers) {
        return err(StatusCode::FORBIDDEN, "only at this Mac");
    }
    let apps: Option<Vec<String>> = body
        .get("mine_only")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let Some(apps) = apps else {
        return err(StatusCode::BAD_REQUEST, "mine_only is a list of apps");
    };
    match crate::server::people::modify(|people| people.keep_for_owner(apps)) {
        Ok(()) => {}
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &format!("persist: {e}")),
    }
    Json(json!({ "status": "ok", "owner": crate::server::people::current().owner })).into_response()
}

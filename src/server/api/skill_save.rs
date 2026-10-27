//! A skill's declared save (`cloud.save`), for a paired device of the same
//! account — the phone that plays the same game on its own and keeps it in
//! step with this Mac over the WebRTC channel (the tunnel is the only way a
//! phone reaches these routes; its peer identity rides `x-linggen-actor-device`).
//!
//!   GET /api/skill-save/{skill}   the save as it is here: `{fingerprint, body, rest_s}`
//!   PUT /api/skill-save/{skill}   `{base, body, over_changes?}` → `{done: saved | stale | in_play, …}`
//!
//! Unlike `skill_sync` (read-only by design: a declaration there cannot say
//! what a device's write means), a `cloud.save` already defines it — the
//! files, the lock, the conflict copies a pull leaves (`account/cloud_save`).
//! Nothing here knows which skill it serves.
//!
//! Same account only: the device must be signed in to the account this Mac is
//! signed in to. Another member's phone paired to the same Mac is refused, and
//! so is an unidentified peer. The account's copy on linggen.dev is not touched
//! here: the Mac's own sync edges (a turn's, the page's) carry a change on.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::account::cloud_save::peer::{self, PeerWrite};
use crate::account::cloud_save::{target, SaveTarget};
use crate::server::{ServerEvent, ServerState};

/// Why a peer may not reach the save.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    MacSignedOut,
    NotIdentified,
    OtherAccount,
}

impl Refused {
    fn word(&self) -> &'static str {
        match self {
            Refused::MacSignedOut => "mac-signed-out",
            Refused::NotIdentified => "not-identified",
            Refused::OtherAccount => "other-account",
        }
    }
}

/// The same-account rule: this Mac's account, and the one the device says it is signed in to.
pub(crate) fn same_account(mac: Option<&str>, device: Option<&str>) -> Result<(), Refused> {
    let mac = mac.filter(|m| !m.is_empty()).ok_or(Refused::MacSignedOut)?;
    let device = device
        .filter(|d| !d.is_empty())
        .ok_or(Refused::NotIdentified)?;
    if device != mac {
        return Err(Refused::OtherAccount);
    }
    Ok(())
}

fn gate(headers: &HeaderMap) -> Result<(), Response> {
    let mac = crate::account::load_account().and_then(|a| a.user_id);
    let device = crate::server::api::pair::actor_for_headers(headers).and_then(|a| a.account);
    same_account(mac.as_deref(), device.as_deref()).map_err(|r| {
        (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "refused": r.word() })),
        )
            .into_response()
    })
}

async fn save_of(state: &Arc<ServerState>, name: &str) -> Result<SaveTarget, Response> {
    let not_found = || (StatusCode::NOT_FOUND, "no save for that skill").into_response();
    let skill = state.skills.get_skill(name).await.ok_or_else(not_found)?;
    target(&skill).ok_or_else(not_found)
}

pub async fn get_save(
    State(state): State<Arc<ServerState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = gate(&headers) {
        return r;
    }
    let save = match save_of(&state, &name).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    match peer::read(&save).await {
        Ok(copy) => Json(copy).into_response(),
        Err(e) => (StatusCode::SERVICE_UNAVAILABLE, format!("{e:#}")).into_response(),
    }
}

#[derive(Deserialize)]
pub struct PutBody {
    /// The fingerprint the device last read here ("" for no save).
    base: String,
    body: serde_json::Value,
    /// The device is replacing changes made here: keep them as copies.
    #[serde(default)]
    over_changes: bool,
}

pub async fn put_save(
    State(state): State<Arc<ServerState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(req): Json<PutBody>,
) -> Response {
    if let Err(r) = gate(&headers) {
        return r;
    }
    let save = match save_of(&state, &name).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let done = match peer::write(&save, &req.base, &req.body, req.over_changes).await {
        Ok(d) => d,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("{e:#}")).into_response(),
    };
    if let PeerWrite::Saved { conflicts, .. } = &done {
        // The page open here reads again (and says a copy was kept, when one was).
        let _ = state.events_tx.send(ServerEvent::SkillSaveChanged {
            skill: save.skill.clone(),
            version: 0,
            conflicts: conflicts.iter().map(|p| p.display().to_string()).collect(),
        });
    }
    Json(done).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_macs_own_account_reaches_the_save() {
        assert_eq!(same_account(Some("u1"), Some("u1")), Ok(()));
        assert_eq!(
            same_account(Some("u1"), Some("u2")),
            Err(Refused::OtherAccount)
        );
        assert_eq!(same_account(Some("u1"), None), Err(Refused::NotIdentified));
        assert_eq!(
            same_account(Some("u1"), Some("")),
            Err(Refused::NotIdentified)
        );
        assert_eq!(same_account(None, Some("u1")), Err(Refused::MacSignedOut));
        assert_eq!(same_account(Some(""), Some("")), Err(Refused::MacSignedOut));
    }
}

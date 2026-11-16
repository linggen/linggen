//! A machine on the LAN asks to share this memory; someone at this machine
//! Allows or Denies. The verbs behind `/api/connect/*` (the routes and their
//! doc are `http/connect/mod.rs`). None checks who is asking: `request` and
//! `status` are open routes, every other verb is guarded in the handler by
//! `local::require_local`, which runs before the op is called.

use serde::Deserialize;
use serde_json::{json, Value};
use std::net::IpAddr;
use std::time::Instant;

use crate::error::MemError;
use crate::handle::Memory;
use crate::http::connect::devices;
use crate::http::connect::{expires_in, Pending, Status};
use crate::http::presence::{Presence, Seen};

#[derive(Deserialize)]
pub struct ConnectRequest {
    pub device_name: String,
    pub device_id: String,
}

fn io_error(e: std::io::Error) -> MemError {
    MemError::internal(anyhow::anyhow!(e))
}

/// A name for the console: printable, trimmed, at most 64 characters.
fn clean_name(raw: &str) -> Option<String> {
    let name: String = raw.chars().filter(|c| !c.is_control()).take(64).collect();
    let name = name.trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// A stable id the asking machine chose: 8–128 of `[A-Za-z0-9._:-]`.
fn valid_device_id(id: &str) -> bool {
    (8..=128).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
}

/// A machine at `peer` asks to connect; rate-limited per address, refused at
/// once if on the denied list, else it becomes the one pending request.
pub async fn request(state: &Memory, peer: IpAddr, req: ConnectRequest) -> Result<Value, MemError> {
    let now = Instant::now();
    if !state.connect.admit(peer, now) {
        return Err(MemError::too_many("too many requests; wait a minute"));
    }
    let name = clean_name(&req.device_name)
        .ok_or_else(|| MemError::bad_request("device_name is required"))?;
    if !valid_device_id(&req.device_id) {
        return Err(MemError::bad_request(
            "device_id must be 8-128 of [A-Za-z0-9._:-]",
        ));
    }
    if devices::is_denied(&state.data_dir, &req.device_id) {
        return Err(MemError::forbidden("this machine was denied"));
    }
    let p = state.connect.open(name, req.device_id, peer, now);
    tracing::info!("connect request from {} ({})", p.device_name, p.ip);
    Ok(json!({ "request_id": p.id, "expires_in": expires_in(&p, now) }))
}

/// Where request `id` stands; `allowed` carries the device token once.
pub async fn status(state: &Memory, id: &str) -> Result<Value, MemError> {
    Ok(status_json(state.connect.status(id, Instant::now())))
}

fn status_json(status: Status) -> Value {
    match status {
        Status::Pending => json!({ "state": "pending" }),
        Status::Denied => json!({ "state": "denied" }),
        Status::Expired => json!({ "state": "expired" }),
        Status::Allowed(Some(token)) => json!({ "state": "allowed", "device_token": token }),
        Status::Allowed(None) => json!({ "state": "allowed" }),
    }
}

fn pending_json(state: &Memory, p: &Pending, now: Instant) -> Value {
    json!({
        "id": p.id,
        "device_name": p.device_name,
        "device_id": p.device_id,
        "ip": p.ip,
        "expires_in": expires_in(p, now),
        "known": devices::is_known(&state.data_dir, &p.device_id),
    })
}

/// The open request, if one waits for a click.
pub async fn pending(state: &Memory) -> Result<Value, MemError> {
    let now = Instant::now();
    let p = state.connect.pending(now);
    Ok(json!({ "pending": p.map(|p| pending_json(state, &p, now)) }))
}

/// The live request with this id, taken off the table to be decided.
fn take(state: &Memory, id: &str) -> Result<Pending, MemError> {
    state
        .connect
        .take(id, Instant::now())
        .ok_or_else(|| MemError::not_found("no such request, or it expired"))
}

/// Pair the machine behind request `id`: write its token into
/// `paired-devices.json`, then hand it to the asker.
pub async fn allow(state: &Memory, id: &str) -> Result<Value, MemError> {
    let p = take(state, id)?;
    let token =
        devices::allow(&state.data_dir, &p.device_id, &p.device_name, p.ip).map_err(io_error)?;
    state.connect.allow(p.id, token, Instant::now());
    tracing::info!("allowed {} ({})", p.device_name, p.ip);
    Ok(json!({ "allowed": p.device_name }))
}

/// Refuse request `id` and put its machine on the denied list.
pub async fn deny(state: &Memory, id: &str) -> Result<Value, MemError> {
    let p = take(state, id)?;
    devices::deny(&state.data_dir, &p.device_id, &p.device_name, p.ip).map_err(io_error)?;
    state.connect.deny(p.id, Instant::now());
    tracing::info!("denied {} ({})", p.device_name, p.ip);
    Ok(json!({ "denied": p.device_name }))
}

/// Every machine this daemon allowed, with what the gate saw of it.
pub async fn list_devices(state: &Memory) -> Result<Value, MemError> {
    let rows = devices::list_allowed(&state.data_dir).map_err(io_error)?;
    Ok(devices_json(&state.presence, &rows, Instant::now()))
}

/// Each machine this daemon allowed, with what the gate saw of it. No secret
/// is in `Allowed`, so none can leak here.
fn devices_json(presence: &Presence, rows: &[devices::Allowed], now: Instant) -> Value {
    let list: Vec<_> = rows
        .iter()
        .map(|a| device_json(a, presence.get(&a.id), now))
        .collect();
    let connected = list.iter().filter(|d| d["online"] == true).count();
    json!({
        "devices": list,
        "summary": { "connected": connected, "total": rows.len() },
    })
}

fn device_json(a: &devices::Allowed, seen: Option<Seen>, now: Instant) -> Value {
    let wall = chrono::Utc::now().timestamp();
    json!({
        "id": a.id,
        "name": a.name,
        "ip": seen.map_or(a.ip, |s| s.ip),
        "allowed_at": a.allowed_at,
        "kind": if seen.is_some_and(|s| s.is_replica()) { "replica" } else { "client" },
        "last_seen": seen.map(|s| wall - s.age_secs(now)),
        "online": seen.is_some_and(|s| s.online(now)),
    })
}

/// Unpair device `id`.
pub async fn revoke(state: &Memory, id: &str) -> Result<Value, MemError> {
    if !devices::revoke(&state.data_dir, id).map_err(io_error)? {
        return Err(MemError::not_found("no such device"));
    }
    Ok(json!({ "revoked": id }))
}

/// The machines that were denied.
pub async fn list_denied(state: &Memory) -> Result<Value, MemError> {
    let rows = devices::list_denied(&state.data_dir).map_err(io_error)?;
    Ok(json!({ "denied": rows }))
}

/// Take `device_id` off the denied list.
pub async fn undeny(state: &Memory, device_id: &str) -> Result<Value, MemError> {
    if !devices::undeny(&state.data_dir, device_id).map_err(io_error)? {
        return Err(MemError::not_found("not on the denied list"));
    }
    Ok(json!({ "undenied": device_id }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_ids_are_checked() {
        assert_eq!(clean_name("  DS242\n").as_deref(), Some("DS242"));
        assert_eq!(clean_name(" \u{7} "), None);
        assert_eq!(clean_name(&"x".repeat(100)).unwrap().len(), 64);
        assert!(valid_device_id("3f2a9c1e-0b7d-4e6a-9f00-1234567890ab"));
        assert!(!valid_device_id("short"));
        assert!(!valid_device_id("has space in it"));
        assert!(!valid_device_id("../../etc/passwd"));
    }

    #[test]
    fn the_status_carries_the_token_only_when_given_one() {
        let v = status_json(Status::Allowed(Some("cafe".into())));
        assert_eq!(v["device_token"], "cafe");
        assert!(status_json(Status::Allowed(None))
            .get("device_token")
            .is_none());
        assert_eq!(status_json(Status::Expired)["state"], "expired");
    }

    #[test]
    fn devices_carry_presence_and_a_summary() {
        let row = |id: &str| devices::Allowed {
            id: id.into(),
            device_id: format!("dev-{id}"),
            name: "luffy-ubuntu".into(),
            ip: "192.168.20.242".parse().unwrap(),
            allowed_at: 1,
        };
        let presence = Presence::default();
        let now = Instant::now();
        presence.record(
            "a",
            "192.168.20.50".parse().unwrap(),
            "/api/sync/changes",
            now,
        );
        let v = devices_json(&presence, &[row("a"), row("b")], now);
        assert_eq!(v["summary"]["connected"], 1);
        assert_eq!(v["summary"]["total"], 2);
        let a = &v["devices"][0];
        assert_eq!(a["online"], true);
        assert_eq!(a["kind"], "replica");
        assert_eq!(a["ip"], "192.168.20.50");
        assert!(a["last_seen"].is_i64());
        assert!(a.get("secret").is_none());
        let b = &v["devices"][1];
        assert_eq!(b["online"], false);
        assert_eq!(b["kind"], "client");
        assert!(b["last_seen"].is_null());
        assert_eq!(b["ip"], "192.168.20.242");
    }
}

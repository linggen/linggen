//! Becoming a replica: this daemon asks the primary to connect (the primary's
//! `/api/connect/request`, the same Allow / Deny flow a person sees there),
//! then follows the request until someone clicks. On Allow the token lands in
//! `.replica.json`; no client ever handles it.

use serde_json::{json, Value};
use std::time::Duration;

use crate::http::state::SharedState;

const FOLLOW_EVERY: Duration = Duration::from_secs(2);
const FOLLOW_FOR: Duration = Duration::from_secs(130);

/// Make this daemon the primary again. Its copy stays; it just stops following.
pub fn become_primary(state: &SharedState) {
    state.replica.update(|f| {
        f.primary = None;
        f.token = None;
        f.request_id = None;
        f.cursor = 0;
        f.online = false;
        f.error = None;
    });
}

/// Ask `primary` (`host:port`) to let this machine follow it.
pub async fn ask(state: &SharedState, primary: String) -> anyhow::Result<()> {
    let device_id = device_id(state);
    let body = json!({ "device_name": host_name(), "device_id": device_id });
    let url = format!("http://{primary}/api/connect/request");
    let resp = super::http().post(&url).json(&body).send().await?;
    let status = resp.status();
    let reply: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let why = reply["error"].as_str().unwrap_or("refused").to_string();
        anyhow::bail!("{primary} said: {why}");
    }
    let rid = reply["data"]["request_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("{primary} opened no request"))?
        .to_string();
    state.replica.update(|f| {
        f.primary = Some(primary.clone());
        f.token = None;
        f.cursor = 0;
        f.request_id = Some(rid.clone());
        f.online = true;
        f.error = None;
    });
    let st = state.clone();
    tokio::spawn(async move { follow(st, primary, rid).await });
    Ok(())
}

fn device_id(state: &SharedState) -> String {
    if let Some(id) = state.replica.snapshot().device_id {
        return id;
    }
    let id = uuid::Uuid::new_v4().to_string();
    state.replica.update(|f| f.device_id = Some(id.clone()));
    id
}

fn host_name() -> String {
    std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "ling-mem".to_string())
}

/// Poll the request until it is decided, then record the outcome.
async fn follow(state: SharedState, primary: String, rid: String) {
    let deadline = tokio::time::Instant::now() + FOLLOW_FOR;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(FOLLOW_EVERY).await;
        if state.replica.snapshot().request_id.as_deref() != Some(rid.as_str()) {
            return; // replaced or cancelled
        }
        let url = format!("http://{primary}/api/connect/status/{rid}");
        let Ok(resp) = super::http().get(&url).send().await else {
            continue;
        };
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if settle(&state, &rid, &body["data"]) {
            return;
        }
    }
    finish(
        &state,
        &rid,
        None,
        Some("no answer on the primary in time".into()),
    );
}

/// True once the request is decided.
fn settle(state: &SharedState, rid: &str, data: &Value) -> bool {
    match data["state"].as_str() {
        Some("allowed") => {
            let token = data["device_token"].as_str().map(str::to_string);
            let err = token
                .is_none()
                .then(|| "approved, but the token was already taken".into());
            finish(state, rid, token, err);
            state.replica.nudge();
            true
        }
        Some("denied") => {
            finish(state, rid, None, Some("denied on the primary".into()));
            true
        }
        Some("expired") => {
            finish(state, rid, None, Some("the request expired".into()));
            true
        }
        _ => false,
    }
}

fn finish(state: &SharedState, rid: &str, token: Option<String>, error: Option<String>) {
    state.replica.update(|f| {
        if f.request_id.as_deref() != Some(rid) {
            return;
        }
        f.request_id = None;
        f.error = error;
        if token.is_some() {
            f.token = token;
        }
    });
}

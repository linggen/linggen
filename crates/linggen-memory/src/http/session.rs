//! `POST /api/memory/session_start` — what every host loads when a session
//! begins: the core rows (who the user is), rendered once, in the daemon, so
//! every host injects the same block.
//!
//! Core only. Preferences (`type = preference`) are not loaded here: they
//! surface by subject through per-turn recall, like any other long-term row.
//! A host that still sends `cwd` or `budget_chars` (built for ling-mem 1.9.0) is
//! answered the same way: unknown fields are ignored.

use super::envelope::{ok, ApiError};
use super::state::SharedState;
use crate::memory::{AccountScope, Filters, Memory, SortOrder, Tier};
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

/// Core is meant to be a handful of rows; the cap keeps a runaway tag from
/// inflating every session.
const CORE_LIMIT: usize = 200;

pub fn router() -> Router<SharedState> {
    Router::new().route("/api/memory/session_start", post(session_start))
}

#[derive(Debug, Default, Deserialize)]
pub struct SessionStartRequest {
    #[serde(default)]
    pub account: Option<String>,
}

async fn session_start(
    State(state): State<SharedState>,
    Json(req): Json<SessionStartRequest>,
) -> Result<Response, ApiError> {
    let account = AccountScope::from_args(req.account, false);
    let core = state
        .store
        .list(
            &Filters {
                tier: Some(Tier::Core),
                account,
                ..Default::default()
            },
            SortOrder::Newest,
            CORE_LIMIT,
            0,
        )
        .await?;
    Ok(ok(SessionStart::build(core).to_json()))
}

fn line(row: &Memory) -> String {
    format!("- {} (id={})", row.content.trim(), row.id)
}

pub struct SessionStart {
    pub core: Vec<Memory>,
    pub block: String,
}

impl SessionStart {
    pub fn build(core: Vec<Memory>) -> Self {
        let block = render(&core);
        Self { core, block }
    }

    pub fn to_json(&self) -> Value {
        let core: Vec<Value> = self
            .core
            .iter()
            .map(|r| {
                let mut v = serde_json::to_value(r).unwrap_or(Value::Null);
                if let Some(o) = v.as_object_mut() {
                    o.remove("vector");
                }
                v
            })
            .collect();
        json!({
            "core": core,
            "block": self.block,
            "chars": self.block.chars().count(),
        })
    }
}

fn render(core: &[Memory]) -> String {
    if core.is_empty() {
        return String::new();
    }
    let rows: Vec<String> = core.iter().map(line).collect();
    format!("## Core memory — who the user is\n\n{}", rows.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{MemoryType, Origin};

    #[test]
    fn the_block_is_core_rows_with_ids() {
        let mut core = Memory::new("Alex — founder", MemoryType::Fact, Origin::User);
        core.tier = Tier::Core;
        let s = SessionStart::build(vec![core.clone()]);
        assert_eq!(
            s.block,
            format!(
                "## Core memory — who the user is\n\n- Alex — founder (id={})",
                core.id
            )
        );
        let json = s.to_json();
        assert_eq!(json["chars"], json!(s.block.chars().count()));
        assert!(json.get("rules").is_none());
        assert!(json["core"][0].get("vector").is_none());
    }

    #[test]
    fn an_old_host_sending_cwd_and_budget_still_parses() {
        let req: SessionStartRequest =
            serde_json::from_value(json!({"cwd": "/u/w/p", "budget_chars": 6000})).unwrap();
        assert!(req.account.is_none());
    }

    #[test]
    fn an_empty_store_renders_nothing() {
        let s = SessionStart::build(vec![]);
        assert_eq!(s.block, "");
    }
}

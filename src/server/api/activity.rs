//! The write door to this machine's activity log.
//!
//! Skills, app pages and scripts have no way into the engine's memory, so this
//! is how they record that the world changed. The engine names no app: `app`
//! is the writing skill's own name, and the verb must be one that skill
//! declares in its `activity-verbs:` — so the set stays closed without the
//! engine knowing any app. The engine's own records never come through here.
//!
//! Gated like every other route: loopback (local skills) passes, LAN callers
//! need their device token. Perception is local by law
//! (`doc/perception-spec.md` §7) — nothing written here leaves the machine.

use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::engine::skill::Skill;
use crate::server::ServerState;

use crate::perception::activity;
use crate::perception::query as activity_query;

/// Why the door refuses `verb` from `app`, or `None` when its skill declares
/// it. `skill` is the installed skill named `app`, if there is one.
fn refusal(skill: Option<&Skill>, app: &str, verb: &str) -> Option<String> {
    let Some(skill) = skill else {
        return Some(format!(
            "no installed skill is named '{app}'; app must be the writing skill's name"
        ));
    };
    if skill.activity_verbs.iter().any(|v| v == verb) {
        return None;
    }
    Some(format!(
        "skill '{app}' does not declare the verb '{verb}'; add it to activity-verbs in its SKILL.md"
    ))
}

#[derive(Deserialize)]
pub(crate) struct RecordBody {
    /// `user`, `yinyue`, `ling`, `system` — the actor, not the device.
    #[serde(default)]
    by: Option<String>,
    /// The writing skill's name.
    app: String,
    /// One of the verbs that skill declares (`activity-verbs:`).
    verb: String,
    /// What it happened to, in the user's terms: a song title, not a path.
    #[serde(default)]
    object: Option<String>,
    #[serde(default)]
    detail: Option<serde_json::Value>,
}

/// POST /api/activity — record one change to this machine's world.
///
/// A change, never a glance (§3). The caller decides what a verb means; the
/// engine checks only that its skill declared it.
///
/// Two things the caller does *not* decide, because both end up inside the
/// resident's prompt: **who** — `by` is a closed set, since it is rendered as
/// the subject of a sentence — and **where** — the device is read from the
/// caller's own token, so a phone's record says the phone rather than this Mac.
/// Length and line breaks are the writer's door's business (`clean`), so every
/// writer gets the same ceiling and none of them can end a bullet early.
pub(crate) async fn record(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Json(body): Json<RecordBody>,
) -> impl IntoResponse {
    if body.app.trim().is_empty() || body.verb.trim().is_empty() {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "app and verb are required" })),
        );
    }
    let skill = state.skills.get_skill(body.app.trim()).await;
    if let Some(error) = refusal(skill.as_ref(), body.app.trim(), body.verb.trim()) {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": error })),
        );
    }
    let by = body.by.as_deref().unwrap_or("user").trim().to_string();
    if !activity::is_actor(&by) {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("by must be one of {:?}", activity::ACTORS),
            })),
        );
    }
    // `detail` is never rendered into a prompt, so it needs no cleaning — but it
    // is written to disk verbatim, and the log is a few days of small lines, not
    // a place to park a payload. Dropped rather than refused: the fact still
    // belongs in the log, and a caller should not lose an activity over a
    // structured extra nobody reads.
    const DETAIL_MAX: usize = 2048;
    let detail = body.detail.filter(|d| {
        let ok = serde_json::to_string(d)
            .map(|s| s.len() <= DETAIL_MAX)
            .unwrap_or(false);
        if !ok {
            tracing::debug!("[perception] dropped an oversized detail from {}", body.app);
        }
        ok
    });
    // A LAN caller carries a device token; loopback does not, and loopback is
    // this Mac.
    let device = crate::server::api::pair::caller_device(&headers)
        .map(|id| crate::server::api::pair::device_name(&id).unwrap_or(id));
    activity::record_from(
        device,
        &by,
        body.app.trim(),
        body.verb.trim(),
        body.object,
        detail,
    );
    (
        axum::http::StatusCode::OK,
        Json(serde_json::json!({ "ok": true })),
    )
}

/// GET /api/activity?since=&until=&app=&verb=&text=&limit=&totals= — the
/// same answer the agent's `recent_activity` tool returns, for a page that
/// wants to show it.
///
/// One reader and one writer for one log: a page rendering its own version of
/// "what happened" from some other source is how two accounts of one day start
/// to disagree.
pub(crate) async fn recent(Query(q): Query<activity_query::Args>) -> impl IntoResponse {
    Json(activity_query::read(&q))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(text: &str) -> Skill {
        crate::extensions::skills::parse_skill_text(
            text,
            crate::engine::skill::SkillSource::Project,
        )
        .unwrap()
    }

    #[test]
    fn the_door_takes_a_declared_verb_and_refuses_the_rest() {
        let notes = skill("---\nname: notes\ndescription: x\nactivity-verbs: [archive]\n---\nBody");
        assert_eq!(refusal(Some(&notes), "notes", "archive"), None);
        let undeclared = refusal(Some(&notes), "notes", "delete").unwrap();
        assert!(undeclared.contains("does not declare the verb 'delete'"), "{undeclared}");
        let engine = refusal(Some(&notes), "notes", "used").unwrap();
        assert!(engine.contains("'used'"), "the engine's verbs are not a skill's: {engine}");
        let stranger = refusal(None, "system", "connect").unwrap();
        assert!(stranger.contains("no installed skill"), "{stranger}");
        let silent = skill("---\nname: quiet\ndescription: x\n---\nBody");
        assert!(refusal(Some(&silent), "quiet", "archive").is_some(), "declares nothing: takes nothing");
    }
}

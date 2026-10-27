//! Raising a declared line: when the skill's `after` mission ends well, its
//! `facts` command says what the line is about (or nothing); code may raise
//! one directly through `POST /api/push/notice`. Either way it joins the
//! queue once per told key.

use std::sync::Arc;

use crate::engine::skill::{NoticeDecl, Skill};
use crate::server::ServerState;

use super::store::{self, Pending};
use super::timing;

/// What a `facts` command (or a notice post) says.
#[derive(Debug, PartialEq)]
pub(crate) struct Raised {
    pub key: String,
    pub facts: serde_json::Value,
    pub route: Option<String>,
    pub until: Option<u64>,
}

/// Read `{key, facts, route?, until?}`. Empty output, `{}` or no key: nothing
/// to say. `until` is unix seconds or an RFC 3339 time.
pub(crate) fn parse(out: &str) -> Option<Raised> {
    let v: serde_json::Value = serde_json::from_str(out.trim()).ok()?;
    let key = v.get("key")?.as_str()?.trim();
    if key.is_empty() || key.len() > 120 {
        return None;
    }
    let facts = v.get("facts").cloned().filter(|f| !f.is_null())?;
    let route = v.get("route").and_then(|r| r.as_str()).map(str::to_string);
    let until = v.get("until").and_then(|u| {
        u.as_u64().or_else(|| {
            chrono::DateTime::parse_from_rfc3339(u.as_str()?)
                .ok()
                .map(|d| d.timestamp().max(0) as u64)
        })
    });
    Some(Raised {
        key: key.to_string(),
        facts,
        route,
        until,
    })
}

/// Queue the line unless the same told key is already waiting.
pub(crate) fn enqueue(app: &str, decl: &NoticeDecl, raised: Raised, now: u64) -> bool {
    let pending = Pending {
        app: app.to_string(),
        line: decl.line.clone(),
        key: raised.key,
        tier: decl.tier,
        facts: raised.facts,
        route: raised.route,
        raised_at: now,
        until: raised
            .until
            .unwrap_or_else(|| timing::default_until(decl.tier, now)),
        words: Default::default(),
        retry_at: Default::default(),
    };
    let told = pending.told_key();
    let added = store::modify(|s| {
        if s.queue.iter().any(|p| p.told_key() == told) {
            return false;
        }
        s.queue.push(pending);
        true
    });
    tracing::info!(
        "[push] {told} raised{}",
        if added {
            ""
        } else {
            " again (already waiting)"
        }
    );
    added
}

/// A mission of a skill finished: raise each line declared `after` it.
pub(crate) async fn after_mission(state: &Arc<ServerState>, mission_id: &str) {
    let Some((skill_name, mission)) = mission_id.split_once(':') else {
        return;
    };
    let Some(skill) = state.skills.get_skill(skill_name).await else {
        return;
    };
    for decl in skill
        .notices
        .iter()
        .filter(|n| n.after.as_deref() == Some(mission))
    {
        raise_from_facts(&skill, decl).await;
    }
}

async fn raise_from_facts(skill: &Skill, decl: &NoticeDecl) {
    let Some(cmd) = decl.facts.as_deref() else {
        return;
    };
    match crate::engine::skill::tools::notice_facts(skill, cmd).await {
        Ok(out) => match parse(&out) {
            Some(r) => {
                enqueue(&skill.name, decl, r, crate::util::now_ts_secs());
            }
            None => tracing::info!("[push] {}.{}: nothing to say", skill.name, decl.line),
        },
        Err(e) => tracing::warn!("[push] {}.{} facts: {e:?}", skill.name, decl.line),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_notice_is_read_with_its_key_facts_route_and_until() {
        let r = parse(r#"{"key":"watch|2026-10-27","facts":[{"about":"AAPL"}],"route":"cfo/invest","until":"2026-10-27T12:00:00-04:00"}"#).unwrap();
        assert_eq!(r.key, "watch|2026-10-27");
        assert_eq!(r.facts, json!([{"about": "AAPL"}]));
        assert_eq!(r.route.as_deref(), Some("cfo/invest"));
        assert_eq!(r.until, Some(1_793_116_800));
    }

    #[test]
    fn nothing_printed_or_no_key_is_nothing_to_say() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("{}"), None);
        assert_eq!(parse(r#"{"facts":[1]}"#), None);
        assert_eq!(parse(r#"{"key":"k"}"#), None);
        assert_eq!(parse("quiet night"), None);
    }
}

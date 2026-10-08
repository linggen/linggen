//! The Mac's tool catalog for the user's other devices (`doc/app-action-spec.md`
//! § Cross-device calls): every installed skill's `remote: true` tools,
//! published as the retained `mac/tools` topic — the mirror of the phone's
//! `phone/tools`. A device's agent offers each as `<app>_<verb>` and calls it
//! through `POST /api/skills/{app}/tools/{name}`.
//!
//! Retained, not queued: the catalog is what this Mac offers, and a newer one
//! makes the older worthless. It is republished whenever the answer changes:
//! checked at startup, after every skill load or reload, and on a slow tick.

use serde_json::{json, Map, Value};
use std::sync::Arc;
use std::time::Duration;

use crate::engine::skill::tools::{remote_tools, tier_of};
use crate::engine::skill::Skill;
use crate::engine::skill_tool::{SkillParamDef, SkillToolDef};
use crate::server::{api, ServerState};

pub const TOPIC: &str = "mac";
pub const OP: &str = "tools";

/// The fallback pace between skill-set changes.
const TICK: Duration = Duration::from_secs(300);

/// Keep `mac/tools` current for as long as the daemon runs.
pub async fn publish_loop(state: Arc<ServerState>) {
    let mut last: Option<Vec<Value>> = None;
    loop {
        let tools = catalog(&state.skills.list_skills().await);
        if last.as_ref() != Some(&tools) {
            publish(&state, &tools);
            last = Some(tools);
        }
        // A load or reload, else the tick (a skill's files can change
        // without either, and the next activation's reload rings late).
        let _ = tokio::time::timeout(TICK, state.skills.changed()).await;
    }
}

/// Retain it for whoever reads later, and hand it to whoever is connected.
fn publish(state: &Arc<ServerState>, tools: &[Value]) {
    let payload = json!({
        "tools": tools,
        "host": crate::perception::host_name(),
        "published_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    });
    api::topic::retain(TOPIC, OP, &payload);
    api::topic::publish_topic(state, TOPIC, OP, payload);
    tracing::info!("[mac_tools] published {} remote tools", tools.len());
}

/// Every skill's remote tools, in a stable order (skill, then declaration).
fn catalog(skills: &[Skill]) -> Vec<Value> {
    let mut sorted: Vec<&Skill> = skills.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    sorted
        .into_iter()
        .flat_map(|s| remote_tools(s).map(move |t| entry(&s.name, t)))
        .collect()
}

/// One tool as the phone's catalog shapes it, plus its wire name.
fn entry(app: &str, tool: &SkillToolDef) -> Value {
    json!({
        "app": app,
        "name": tool.name,
        "wire_name": wire_name(app, &tool.name),
        "description": tool.description,
        "params": params(tool),
        "tier": tier_of(tool).to_string(),
    })
}

fn params(tool: &SkillToolDef) -> Value {
    let mut names: Vec<&String> = tool.args.keys().collect();
    names.sort();
    let map: Map<String, Value> = names
        .into_iter()
        .map(|n| (n.clone(), param(&tool.args[n])))
        .collect();
    Value::Object(map)
}

fn param(p: &SkillParamDef) -> Value {
    let mut v = json!({
        "type": p.param_type,
        "description": p.description,
        "required": p.required,
    });
    if let Some(d) = &p.default {
        v["default"] = d.clone();
    }
    if let Some(items) = &p.items {
        v["items"] = items.clone();
    }
    v
}

/// `<app>_<verb>`: `dj` + `ListLibrary` → `dj_list_library`. Underscores
/// only — model function names and topic ops both refuse dots.
pub fn wire_name(app: &str, tool: &str) -> String {
    format!("{}_{}", snake(app), snake(tool))
}

/// `ListLibrary` → `list_library`, `GetURL` → `get_url`, `apple-shifu` →
/// `apple_shifu`. Anything outside `[a-z0-9]` becomes one underscore.
fn snake(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_ascii_alphanumeric() {
            push_break(&mut out);
            continue;
        }
        if c.is_ascii_uppercase() && starts_word(&chars, i) {
            push_break(&mut out);
        }
        out.push(c.to_ascii_lowercase());
    }
    out.trim_matches('_').to_string()
}

/// An uppercase letter opens a word after a lowercase letter or digit, or
/// when it ends an acronym (`URLParser` → `url_parser`).
fn starts_word(chars: &[char], i: usize) -> bool {
    let Some(&prev) = i.checked_sub(1).and_then(|p| chars.get(p)) else {
        return false;
    };
    if prev.is_ascii_lowercase() || prev.is_ascii_digit() {
        return true;
    }
    let next_lower = chars.get(i + 1).is_some_and(|n| n.is_ascii_lowercase());
    prev.is_ascii_uppercase() && next_lower
}

fn push_break(out: &mut String) {
    if !out.is_empty() && !out.ends_with('_') {
        out.push('_');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::skill::SkillSource;

    #[test]
    fn wire_names_are_app_then_snake_verb() {
        assert_eq!(wire_name("dj", "ListLibrary"), "dj_list_library");
        assert_eq!(wire_name("dj", "AddToPlaylist"), "dj_add_to_playlist");
        assert_eq!(wire_name("dj", "RemoveFromPhone"), "dj_remove_from_phone");
        assert_eq!(wire_name("apple-shifu", "Scan"), "apple_shifu_scan");
        assert_eq!(wire_name("cfo", "already_snake"), "cfo_already_snake");
        assert_eq!(wire_name("x", "GetURL"), "x_get_url");
        assert_eq!(wire_name("x", "URLParser"), "x_url_parser");
        assert_eq!(wire_name("x", "quests.stamp"), "x_quests_stamp");
        assert_eq!(wire_name("x", "Step2Go"), "x_step2_go");
    }

    #[test]
    fn wire_names_fit_the_topic_op_charset() {
        let w = wire_name("Apple Shifu", "Do--It.Now");
        assert!(w
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'));
        assert!(!w.contains("__"), "{w}");
    }

    fn skill(dir: &std::path::Path) -> Skill {
        let text = format!(
            r#"---
name: zz-remote
description: t
permission:
  paths:
    - {{ path: "{}", mode: edit }}
tools:
  - name: ListThings
    description: Reads.
    cmd: "echo {{{{query}}}}"
    tier: read
    remote: true
    args:
      query: {{ type: string, description: Optional filter. }}
      limit: {{ type: integer, required: true, description: Rows. }}
  - name: Local
    description: Mac only.
    cmd: "echo local"
    tier: read
---
body
"#,
            dir.to_string_lossy()
        );
        let mut s = crate::extensions::skills::parse_skill_text(&text, SkillSource::Project)
            .expect("parses");
        s.skill_dir = Some(dir.to_path_buf());
        s
    }

    #[test]
    fn the_catalog_lists_remote_tools_in_the_phone_shape() {
        let dir = tempfile::tempdir().unwrap();
        let tools = catalog(&[skill(dir.path())]);
        assert_eq!(tools.len(), 1);
        let t = &tools[0];
        assert_eq!(t["app"], "zz-remote");
        assert_eq!(t["name"], "ListThings");
        assert_eq!(t["wire_name"], "zz_remote_list_things");
        assert_eq!(t["tier"], "read");
        assert_eq!(t["params"]["limit"]["type"], "integer");
        assert_eq!(t["params"]["limit"]["required"], true);
        assert_eq!(t["params"]["query"]["description"], "Optional filter.");
    }
}

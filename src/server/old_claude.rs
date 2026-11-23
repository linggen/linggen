//! A Claude session from 2.5.1 and before: one the Claude Agent SDK hosted
//! (`SessionMeta::is_old_claude`). That runtime is gone, so the session
//! is a read-only record — it opens, renames, clears and deletes as any
//! session does, takes no turn, and offers one way on: a new Linggen session
//! in the same folder that starts with its conversation
//! (`doc/session-spec.md` § Old Claude sessions).
//!
//! This module holds the reason every door gives and the continue. The one
//! check every native turn passes is `refuse_foreign_runtime`
//! (`engine/agent/mod.rs`).

use crate::server::ServerState;
use crate::state_fs::sessions::{ChatMsg, SessionMeta, COMPACTION_SENDER};
use crate::state_fs::RowAuthor;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Deserialize;
use std::sync::Arc;

/// What a door that refuses an old Claude session says (the page says its own line).
pub(crate) const READ_ONLY: &str =
    "This Claude session is read-only. Continue it in a new Linggen session.";

/// How much of the old conversation the new session starts with: its newest
/// lines, whole, up to this many characters (about ten thousand tokens).
const CARRY_CHARS: usize = 40_000;
/// A single line longer than this is cut to its opening and its closing, so
/// one pasted log cannot fill the carry; a reply's last paragraph is what it
/// concluded, so the cut keeps more of the end.
const LINE_CHARS: usize = 4_000;
const LINE_HEAD_CHARS: usize = 1_000;
const MIDDLE_LEFT_OUT: &str = "\n[…]\n";

#[derive(Deserialize)]
pub(crate) struct ContinueRequest {
    session_id: String,
}

/// POST /api/sessions/continue — a new Linggen session in the old Claude
/// session's folder, on a Claude model when one can run ([`model_for`]),
/// whose first row carries the old conversation ([`carried`]) and names the
/// model. The old session is left as it was. Returns `{id, cwd, model}`.
pub(crate) async fn continue_api(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<ContinueRequest>,
) -> impl IntoResponse {
    let store = &state.manager.global_sessions;
    let Ok(Some(old)) = store.get_session_meta(&req.session_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !old.is_old_claude() {
        return (
            StatusCode::BAD_REQUEST,
            "Only an old Claude session is continued this way.",
        )
            .into_response();
    }
    let rows = store.get_chat_history(&old.id).unwrap_or_default();
    let models = state.manager.models.read().await.clone();
    let config = state.manager.get_config_snapshot().await;
    let chain: Vec<String> = config
        .routing
        .default_models
        .iter()
        .cloned()
        .chain(config.chat_models().map(|m| m.id.clone()))
        .collect();
    let pinned = old.agents.first().and_then(|m| m.model.as_deref());
    let model = model_for(&models, pinned, &chain);
    let meta = continued_meta(&old, model.clone());
    // The model it will run on, named: the one chosen, else what the default
    // chain gives its lead now.
    let named = match model {
        Some(id) => id,
        None => {
            let lead = meta.agents.first().map(|m| m.id.as_str()).unwrap_or("ling");
            state
                .manager
                .agent_model_id(std::path::Path::new(&meta.cwd), lead)
                .await
                .unwrap_or_else(|| "the default model".to_string())
        }
    };
    let seed = seed_row(&meta, carried(&old.title, &rows, &named));
    if store.add_session(&meta).is_err() || store.add_chat_message(&meta.id, &seed).is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    Json(serde_json::json!({ "id": meta.id, "cwd": meta.cwd, "model": named })).into_response()
}

/// Not asked through `model_choice::resolve`: a continued Claude session
/// stays on a Claude model, which `resolve` has no way to say (`first_claude`
/// is its own question, kept in `model_choice`).
///
/// The model a continued session asks for: the one the old session pinned
/// when this engine has it; else the first Anthropic model of `chain` that
/// can run now (a Claude conversation goes on on a Claude model); else none,
/// and the default chain decides.
fn model_for(
    models: &crate::provider::models::ModelManager,
    pinned: Option<&str>,
    chain: &[String],
) -> Option<String> {
    pinned
        .filter(|id| crate::engine::model_choice::is_here(models, id))
        .map(str::to_string)
        .or_else(|| crate::engine::model_choice::first_claude(models, chain))
}

/// The new session: the old one's folder and person, an ordinary chat's
/// lead (`members::default_members`) on `model` (none: the default chain).
/// Built like `api::sessions::create_session` builds a plain chat.
fn continued_meta(old: &SessionMeta, model: Option<String>) -> SessionMeta {
    let mut agents = crate::server::chat::members::default_members(None);
    if let Some(lead) = agents.first_mut() {
        lead.model = model;
    }
    let mut meta = SessionMeta {
        id: format!(
            "sess-{}-{}",
            crate::util::now_ts_secs(),
            &uuid::Uuid::new_v4().to_string()[..8]
        ),
        title: format!("{} (continued)", old.title),
        created_at: crate::util::now_ts_secs(),
        creator: "user".into(),
        agents,
        user_id: old.user_id.clone(),
        title_locked: true,
        ..Default::default()
    };
    meta.set_cwd(&old.cwd);
    meta.settle_cwd();
    // It goes on with coding work: a Coding session, as `work: code` makes one.
    let _ = crate::engine::session_spec::ask_work(&mut meta, "code");
    meta
}

/// The carried conversation as one system note every member reads (a
/// compaction row: the thread starts there, `doc/shared-session-spec.md`).
fn seed_row(meta: &SessionMeta, content: String) -> ChatMsg {
    ChatMsg {
        agent_id: meta
            .agents
            .first()
            .map(|m| m.id.clone())
            .unwrap_or_default(),
        from_id: COMPACTION_SENDER.to_string(),
        to_id: "user".to_string(),
        content,
        timestamp: meta.created_at,
        is_observation: false,
        client_id: None,
        failed: false,
        via: None,
        via_note: None,
        images: Vec::new(),
        run_id: None,
        place: None,
        heard: false,
    }
}

/// What the new session starts with: the person's messages and Claude's
/// replies, newest kept, within [`CARRY_CHARS`] characters (not bytes: a
/// Chinese line costs what an English one does). The header names the model
/// the session runs on and says what is left behind.
pub(crate) fn carried(title: &str, rows: &[ChatMsg], model: &str) -> String {
    let lines: Vec<String> = rows
        .iter()
        .filter(|row| !row.is_observation && !row.content.trim().is_empty())
        .filter_map(|row| {
            let who = match row.author() {
                RowAuthor::Person => "Person",
                RowAuthor::Agent(_) => "Claude",
                _ => return None,
            };
            Some(format!("{who}: {}", cut(row.content.trim())))
        })
        .collect();
    let mut kept = Vec::new();
    let mut used = 0;
    for line in lines.iter().rev() {
        let chars = line.chars().count();
        if used + chars > CARRY_CHARS && !kept.is_empty() {
            break;
        }
        used += chars;
        kept.push(line.as_str());
    }
    kept.reverse();
    let mut text = format!(
        "[Continued from the Claude session \"{title}\", now on {model} — {} of its {} messages carried]\n\n\
         This session goes on from a Claude session that is now read-only. \
         Left behind: tool calls and their results, pictures, and subagents' output; \
         the files they touched are still in the folder. \
         Its conversation, as it was:\n\n",
        kept.len(),
        lines.len()
    );
    text.push_str(&kept.join("\n\n"));
    text
}

/// A line within [`LINE_CHARS`]: whole, or its opening and its closing with
/// the middle left out.
fn cut(text: &str) -> String {
    let count = text.chars().count();
    if count <= LINE_CHARS {
        return text.to_string();
    }
    let head: String = text.chars().take(LINE_HEAD_CHARS).collect();
    let tail: String = text.chars().skip(count - (LINE_CHARS - LINE_HEAD_CHARS)).collect();
    format!("{head}{MIDDLE_LEFT_OUT}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(from: &str, content: &str, observation: bool) -> ChatMsg {
        ChatMsg {
            agent_id: "claude".into(),
            from_id: from.into(),
            to_id: "user".into(),
            content: content.into(),
            timestamp: 1,
            is_observation: observation,
            client_id: None,
            failed: false,
            via: None,
            via_note: None,
            images: Vec::new(),
            run_id: None,
            place: None,
            heard: false,
        }
    }

    /// The carry keeps the conversation and leaves out tool rows.
    #[test]
    fn an_old_claude_session_carries_its_conversation_not_its_tools() {
        let rows = [
            row("user", "Rename the ferry module", false),
            row("claude", r#"{"type":"tool","tool":"Edit"}"#, true),
            row("system", "Tool Edit: done", true),
            row("claude", "Renamed it to boats.rs.", false),
        ];
        let text = carried("Ferry", &rows, "claude-test");
        assert!(text.contains("Person: Rename the ferry module"), "{text}");
        assert!(text.contains("Claude: Renamed it to boats.rs."), "{text}");
        assert!(!text.contains("Tool Edit"), "{text}");
        assert!(text.contains("2 of its 2 messages carried"), "{text}");
    }

    /// What is left behind is said, and so is the model the session runs on.
    #[test]
    fn the_carried_row_names_its_model_and_what_was_left_behind() {
        let text = carried("Ferry", &[row("user", "Hi", false)], "claude-test-model");
        let header = text.lines().next().unwrap();
        assert!(header.contains("claude-test-model"), "{header}");
        for left in ["tool calls", "pictures", "subagents"] {
            assert!(text.contains(left), "{left}: {text}");
        }
    }

    /// A long conversation keeps its newest lines within the budget.
    #[test]
    fn a_long_old_conversation_keeps_its_newest_lines() {
        let rows: Vec<ChatMsg> = (0..40)
            .map(|i| row("user", &format!("line {i} {}", "x".repeat(3000)), false))
            .collect();
        let text = carried("Long", &rows, "claude-test");
        assert!(text.contains("line 39"));
        assert!(!text.contains("line 0 "));
        assert!(text.chars().count() < CARRY_CHARS + 1000);
    }

    /// The budget is characters: Chinese text carries as many lines as English.
    #[test]
    fn chinese_and_english_carry_the_same_number_of_characters() {
        let carried_of = |filler: &str| {
            let rows: Vec<ChatMsg> = (0..40)
                .map(|i| row("user", &format!("{i}-{}", filler.repeat(3000)), false))
                .collect();
            carried("Long", &rows, "claude-test")
        };
        let english = carried_of("x");
        let chinese = carried_of("好");
        let count = |text: &str| text.lines().next().unwrap().split(" of its").next().unwrap().to_string();
        let kept = |text: &str| count(text).rsplit(' ').next().unwrap().parse::<usize>().unwrap();
        assert_eq!(kept(&english), kept(&chinese), "{english:.200} / {chinese:.200}");
        assert!(kept(&chinese) < 40, "the budget was not reached");
    }

    /// A long reply keeps its opening and its closing, not only its opening.
    #[test]
    fn a_long_reply_keeps_both_its_ends() {
        let reply = format!("The plan: rename.{}Done: boats.rs is in place.", " middle".repeat(2000));
        let text = carried("Ferry", &[row("claude", &reply, false)], "claude-test");
        assert!(text.contains("The plan: rename."), "{text:.300}");
        assert!(text.contains("Done: boats.rs is in place."), "the closing was cut");
        let line = text.lines().last().unwrap();
        assert!(line.chars().count() <= LINE_CHARS + 20, "{}", line.chars().count());
    }

    fn anthropic(id: &str, key: Option<&str>) -> crate::config::ModelConfig {
        serde_json::from_value(serde_json::json!({
            "id": id, "provider": "anthropic", "url": "https://api.anthropic.com",
            "model": id, "api_key": key,
        }))
        .unwrap()
    }

    fn manager(configs: Vec<crate::config::ModelConfig>) -> crate::provider::models::ModelManager {
        crate::provider::models::ModelManager::new_with_credentials(
            configs,
            &crate::credentials::Credentials::default(),
        )
    }

    fn chain(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    /// Unpinned: the first Anthropic model that can run, from the defaults then
    /// the configured models. (A model with no key is passed over — that is
    /// `model_choice`'s own test: a key in the environment would skew it here.)
    #[test]
    fn an_unpinned_old_session_continues_on_the_first_claude_model_that_can_run() {
        let models = manager(vec![
            anthropic("claude-b", Some("sk-test-b")),
            anthropic("claude-a", Some("sk-test-a")),
        ]);
        let picked = model_for(&models, None, &chain(&["gpt-6.2-sol", "claude-b", "claude-a"]));
        assert_eq!(picked.as_deref(), Some("claude-b"));
        // A model the defaults do not list is found among the configured ones.
        let picked = model_for(&models, None, &chain(&["gpt-6.2-sol", "claude-a"]));
        assert_eq!(picked.as_deref(), Some("claude-a"));
    }

    /// Unpinned and no Claude model can run: none is asked for, the default
    /// chain decides (and the new session says which).
    #[test]
    fn an_unpinned_old_session_with_no_claude_model_takes_the_default() {
        let models = manager(vec![]);
        assert_eq!(model_for(&models, None, &chain(&["gpt-6.2-sol", "gpt-6.1-terra"])), None);
    }

    /// Pinned and here: the pin carries over, a Claude model or not.
    #[test]
    fn a_pinned_old_session_keeps_its_model_when_it_is_here() {
        let models = manager(vec![anthropic("claude-a", Some("sk-test-a")), anthropic("claude-b", Some("sk-test-b"))]);
        let picked = model_for(&models, Some("claude-a"), &chain(&["claude-b"]));
        assert_eq!(picked.as_deref(), Some("claude-a"));
        // Pinned but gone: it falls to the first Claude model that can run.
        let picked = model_for(&models, Some("claude-gone"), &chain(&["claude-b"]));
        assert_eq!(picked.as_deref(), Some("claude-b"));
    }

    /// The line every door gives is one short line.
    #[test]
    fn the_refusal_line_is_twenty_words_or_fewer() {
        assert!(READ_ONLY.split_whitespace().count() <= 20, "{READ_ONLY}");
    }
}

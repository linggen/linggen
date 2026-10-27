//! Her words for a pushed line: one model call, no tools. Who she is (the
//! same `agents/yinyue.md` the phone assembles her from), the skill's prompt
//! for the line — the phone's prompt shape, so the words are hers either way
//! — and the facts as data, in the phone's language. She answers
//! `{"title", "text"}`; anything else is no line, and nothing goes.

use futures_util::StreamExt;
use std::sync::Arc;

use crate::engine::skill::{NoticeDecl, Skill};
use crate::message::ChatMessage;
use crate::provider::models::StreamChunk;
use crate::server::ServerState;

const TITLE_MAX: usize = 60;
const TEXT_MAX: usize = 320;
const COMPANION: &str = crate::engine::agent::COMPANION_AGENT_ID;

fn message(role: &str, content: String) -> ChatMessage {
    ChatMessage {
        role: role.to_string(),
        content,
        thinking: None,
        images: Vec::new(),
        cache_control: None,
        tool_calls: Vec::new(),
        tool_call_id: None,
        name: None,
    }
}

/// The two messages: her soul + the line's job + the language, then the facts.
pub(crate) fn messages(
    soul: &str,
    job: &str,
    language: &str,
    facts: &serde_json::Value,
) -> Vec<ChatMessage> {
    let lang = crate::language::display_name(language);
    vec![
        message(
            "system",
            format!(
                "{soul}\n\n{}\n\nWrite the title and the text in {lang}.",
                job.trim()
            ),
        ),
        message(
            "user",
            format!("The facts. Use only these; add nothing.\n{facts}"),
        ),
    ]
}

/// Her reply as (title, text), or None when it isn't a line she can say —
/// the same reading the phone gives her (`CfoLines.lineWords`).
pub(crate) fn words(reply: &str) -> Option<(String, String)> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    if end <= start {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&reply[start..=end]).ok()?;
    let one = |k: &str| {
        v.get(k)
            .map(|s| {
                s.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| s.to_string())
            })
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let (title, text) = (one("title"), one("text"));
    let fits = !title.is_empty()
        && !text.is_empty()
        && title.chars().count() <= TITLE_MAX
        && text.chars().count() <= TEXT_MAX;
    fits.then_some((title, text))
}

/// Write the line. Err carries why not, for the log; the caller retries
/// on its next tick until the line's `until`.
pub(crate) async fn write(
    state: &Arc<ServerState>,
    skill: &Skill,
    decl: &NoticeDecl,
    language: &str,
    facts: &serde_json::Value,
) -> Result<(String, String), String> {
    let job = std::fs::read_to_string(prompt_path(skill, decl))
        .map_err(|e| format!("prompt {}: {e}", decl.prompt))?;
    let soul = her_soul(state).await?;
    let model = her_model(state).await.ok_or("no model for her")?;
    let msgs = messages(&soul, &job, language, facts);
    let reply = ask(state, &model, &msgs, skill.product.as_deref()).await?;
    words(&reply).ok_or_else(|| "her reply was not a line".to_string())
}

fn prompt_path(skill: &Skill, decl: &NoticeDecl) -> std::path::PathBuf {
    let dir = skill
        .skill_dir
        .clone()
        .unwrap_or_else(|| crate::paths::global_skills_dir().join(&skill.name));
    dir.join(&decl.prompt)
}

fn her_root() -> std::path::PathBuf {
    crate::util::resolve_path(std::path::Path::new("~/.linggen"))
}

async fn her_soul(state: &Arc<ServerState>) -> Result<String, String> {
    let spec = state
        .manager
        .agents
        .find(&her_root(), COMPANION)
        .await
        .map_err(|e| format!("{e:#}"))?
        .ok_or("her spec is missing")?;
    let personality = spec.spec.personality.clone().unwrap_or_default();
    Ok(crate::engine::prompt::soul_text(
        &personality,
        spec.system_prompt.trim(),
    ))
}

/// The model her turns run on: her pet setting when this Mac has it, else
/// her own chain.
async fn her_model(state: &Arc<ServerState>) -> Option<String> {
    let setting = state.manager.get_config_snapshot().await.pet.model.clone();
    let models = state.manager.models.read().await.clone();
    if let Some(m) =
        crate::server::resident::resolve_pet_model(&setting).and_then(|m| models.resolve_id(&m))
    {
        return Some(m);
    }
    state.manager.agent_model_id(&her_root(), COMPANION).await
}

async fn ask(
    state: &Arc<ServerState>,
    model: &str,
    msgs: &[ChatMessage],
    app: Option<&str>,
) -> Result<String, String> {
    let models = state.manager.models.read().await.clone();
    let mut stream = models
        .chat_text_stream(model, msgs, Some("low"), app)
        .await
        .map_err(|e| format!("{e:#}"))?;
    let mut text = String::new();
    while let Some(chunk) = stream.next().await {
        if let StreamChunk::Token(t) = chunk.map_err(|e| format!("{e:#}"))? {
            text.push_str(&t);
        }
    }
    // A reasoning model's thinking is not her line.
    Ok(match text.rfind("</think>") {
        Some(i) => text[i + "</think>".len()..].to_string(),
        None => text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn her_json_is_read_as_title_and_text() {
        let r = "Here:\n{\"title\": \"苹果  财报\", \"text\": \"营收 增长\\n了 8%。\"}";
        assert_eq!(
            words(r),
            Some(("苹果 财报".into(), "营收 增长 了 8%。".into()))
        );
    }

    #[test]
    fn no_json_or_an_empty_or_long_field_is_no_line() {
        assert_eq!(words("SILENT"), None);
        assert_eq!(words("{\"title\": \"\", \"text\": \"x\"}"), None);
        let long = "x".repeat(400);
        assert_eq!(
            words(&format!("{{\"title\": \"t\", \"text\": \"{long}\"}}")),
            None
        );
    }

    #[test]
    fn the_facts_go_as_data_and_the_language_is_named() {
        let m = messages("SOUL", " job ", "zh-Hans", &serde_json::json!([{"a": 1}]));
        assert!(m[0].content.starts_with("SOUL\n\njob\n\n"));
        assert!(m[0].content.ends_with("in Simplified Chinese."));
        assert_eq!(
            m[1].content,
            "The facts. Use only these; add nothing.\n[{\"a\":1}]"
        );
    }
}

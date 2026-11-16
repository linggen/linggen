//! The speaking agent's own notes, beside the turn.
//!
//! Built per request, like the room ("Right now"), never kept in the thread:
//! the notes are the agent's alone, and a kept row is read by every member
//! of a table. Shown only when the turn offers `note_done`, so an agent sees
//! its notes exactly where it can finish them.

use crate::engine::AgentEngine;
use std::collections::HashSet;

impl AgentEngine {
    /// This turn's notes block; `None` when the turn holds no note tools or
    /// the agent has no open notes.
    pub(super) fn notes_block(&self, allowed_tools: Option<&HashSet<String>>) -> Option<String> {
        let done = crate::notes::DONE_TOOL;
        let offered = allowed_tools.map_or_else(
            || self.holds_own_note_tool(None, done),
            |s| s.contains(done),
        );
        if !offered || self.is_withheld(done) {
            return None;
        }
        let agent = self.agent_id.as_deref()?;
        let book = crate::notes::read(agent);
        let host = crate::perception::host_name();
        let lines = crate::notes::shown::lines(book.notes(), &chrono::Local::now(), &host)?;
        let block = self.prompt_store.render_or_fallback(
            crate::prompts::keys::SYSTEM_NOTES_BLOCK,
            &[("lines", &lines)],
        );
        Some(block.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::prompt::PromptPurpose;
    use crate::engine::{AgentEngine, AgentRole, EngineConfig, InterfaceMode};
    use crate::message::ChatMessage;

    const YINYUE: &str = include_str!("../../../../../agents/yinyue.md");
    const LING: &str = include_str!("../../../../../agents/ling.md");

    fn engine(agent: &str, spec_text: &str) -> AgentEngine {
        let cfg = EngineConfig::from_app_config(
            &crate::config::Config::default(),
            std::env::temp_dir(),
            InterfaceMode::Web,
        );
        let models =
            std::sync::Arc::new(crate::provider::models::ModelManager::new_with_credentials(
                Vec::new(),
                &crate::credentials::Credentials::default(),
            ));
        let mut engine = AgentEngine::new(cfg, models, "m".to_string(), AgentRole::Lead).unwrap();
        let (spec, body) = crate::extensions::agents::parse_agent_markdown(spec_text).unwrap();
        engine.set_spec(agent.to_string(), spec, body);
        engine.prompt_profile = crate::engine::prompt::profile::PromptProfile::owner();
        engine.prompt_profile.include_memory = false;
        engine.prompt_profile.include_core = false;
        engine.chat_history = vec![ChatMessage::new("user", "hi")];
        engine
    }

    fn notes_in(engine: &mut AgentEngine) -> (Option<String>, Vec<String>) {
        let (messages, allowed, _) =
            engine.prepare_loop_messages("hi", true, PromptPurpose::Preview);
        let block = messages
            .iter()
            .find(|m| m.role == "system" && m.content.starts_with("# Your notes"))
            .map(|m| m.content.clone());
        let offered = engine
            .offered_tool_definitions(allowed.as_ref())
            .iter()
            .filter_map(|d| d.pointer("/function/name")?.as_str().map(String::from))
            .collect();
        (block, offered)
    }

    fn clear(agent: &str) {
        let _ = std::fs::remove_file(crate::paths::notes_dir().join(format!("{agent}.json")));
    }

    #[test]
    fn her_open_notes_ride_beside_the_turn_and_the_turn_ends_on_her_words() {
        let agent = "test-notes-block-hers";
        clear(agent);
        let mut e = engine(agent, YINYUE);
        let (none, offered) = notes_in(&mut e);
        assert_eq!(none, None, "no notes, no block");
        assert!(
            offered.iter().any(|t| t == "note_write") && offered.iter().any(|t| t == "note_done")
        );
        crate::notes::change(agent, |b| {
            b.write(
                "the parcel arrives tomorrow",
                None,
                "m",
                crate::util::now_ts_secs(),
            )
        })
        .unwrap();
        let (messages, _, _) = e.prepare_loop_messages("hi", true, PromptPurpose::Preview);
        let block = messages
            .iter()
            .position(|m| m.content.starts_with("# Your notes"))
            .unwrap();
        assert!(messages[block]
            .content
            .contains("- n1 · written just now on m: the parcel arrives tomorrow"));
        assert!(block < messages.len() - 1);
        assert_eq!(messages.last().unwrap().content, "hi");
        assert!(
            !e.chat_history
                .iter()
                .any(|m| m.content.contains("# Your notes")),
            "never kept"
        );
        clear(agent);
    }

    /// An agent that is unrestricted and does not list the note tools has
    /// none: they are never part of `*`.
    #[test]
    fn a_bare_wildcard_agent_has_no_notes_and_is_shown_none() {
        const WORKER: &str = "---\nname: worker\ndescription: d\ntools: [\"*\"]\n---\nWork.";
        let agent = "test-notes-block-worker";
        crate::notes::change(agent, |b| b.write("planted", None, "m", 1)).unwrap();
        let mut e = engine(agent, WORKER);
        let (block, offered) = notes_in(&mut e);
        assert_eq!(block, None);
        assert!(
            !offered.iter().any(|t| crate::notes::is_note_tool(t)),
            "{offered:?}"
        );
        clear(agent);
    }

    /// Ling is unrestricted and lists the note tools: he is offered them and
    /// his open notes ride beside his next turn — and not hers.
    #[test]
    fn ling_holds_notes_beside_the_unrestricted_set_and_they_are_his_alone() {
        let (his, hers) = ("test-notes-block-ling", "test-notes-block-ling-hers");
        clear(his);
        clear(hers);
        let mut ling = engine(his, LING);
        assert!(ling.allowed_tool_names().is_none(), "his set stays unrestricted");
        let (none, offered) = notes_in(&mut ling);
        assert_eq!(none, None, "no notes, no block");
        assert!(
            offered.iter().any(|t| t == "note_write") && offered.iter().any(|t| t == "note_done")
        );
        assert!(offered.iter().any(|t| t == "Bash"), "the rest of his set is still there");
        let now = crate::util::now_ts_secs();
        crate::notes::change(his, |b| b.write("ask about the invoice", None, "m", now)).unwrap();
        let (block, _) = notes_in(&mut ling);
        assert!(block.unwrap().contains(": ask about the invoice"));
        // Hers do not reach him, nor his her: one store per agent.
        crate::notes::change(hers, |b| b.write("the parcel", None, "m", now)).unwrap();
        let (block, _) = notes_in(&mut ling);
        let block = block.unwrap();
        assert!(!block.contains("the parcel"), "{block}");
        let mut her = engine(hers, YINYUE);
        let (block, _) = notes_in(&mut her);
        let block = block.unwrap();
        assert!(block.contains("the parcel") && !block.contains("the invoice"), "{block}");
        clear(his);
        clear(hers);
    }

    /// Her notes follow her to Ling's own chat, whose tool set is his `*`.
    #[test]
    fn her_notes_follow_her_into_lings_unrestricted_chat() {
        let agent = "test-notes-block-hers-at-his";
        clear(agent);
        crate::notes::change(agent, |b| b.write("the parcel", None, "m", 1)).unwrap();
        let mut her = engine(agent, YINYUE);
        let (ling_spec, _) = crate::extensions::agents::parse_agent_markdown(LING).unwrap();
        her.session_lead = Some("ling".into());
        her.session_tools = Some(ling_spec.tools);
        assert!(her.allowed_tool_names().is_none(), "the lead's set: everything");
        let (block, offered) = notes_in(&mut her);
        assert!(block.unwrap().contains(": the parcel"));
        assert!(offered.iter().any(|t| t == "note_done"), "{offered:?}");
        clear(agent);
    }

    /// A stranger's session of Ling (a proxy room) is never shown the owner's
    /// notes, nor offered the tools.
    #[test]
    fn a_proxy_rooms_ling_has_no_notes() {
        let agent = "test-notes-block-room";
        crate::notes::change(agent, |b| b.write("private", None, "m", 1)).unwrap();
        let mut e = engine(agent, LING);
        e.prompt_profile.consumer_frame = true;
        let (block, offered) = notes_in(&mut e);
        assert_eq!(block, None);
        assert!(!offered.iter().any(|t| crate::notes::is_note_tool(t)), "{offered:?}");
        clear(agent);
    }
}

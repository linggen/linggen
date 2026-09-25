//! Where the speaking agent is — the `## Where you are` block.
//!
//! An agent's soul (`agents/<id>.md`) says who it is, the same everywhere.
//! Its place says where it stands this turn: which surface, who is there,
//! what it can do. See `doc/persona-design.md`.
//!
//! Two sources, one block:
//! - **Engine surfaces** — the agent's own session, or a guest seat at
//!   someone else's table. An app's own agent has no engine block: the app's
//!   SKILL.md is its place. Their text is a readable file under
//!   `agents/places/`, embedded at build time; its frontmatter names the
//!   agent and the surface it is for.
//! - **Skills** — a skill declares `place:` per agent in its SKILL.md for
//!   the agents that come to its chat as guests; the declared text replaces
//!   the engine's guest block. The engine names no app and no agent: it
//!   matches ids.

use crate::engine::skill::record::Places;
use rust_embed::Embed;
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Embed)]
#[folder = "agents/places/"]
struct PlaceAssets;

/// Where an engine is speaking from, as far as the engine can tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Surface {
    /// The agent's own session with no app running: the main chat, or the
    /// companion's own thread.
    Home,
    /// The agent runs an app skill in its own session.
    App,
    /// A guest at another session's table.
    Guest,
}

impl Surface {
    fn key(self) -> &'static str {
        match self {
            Surface::Home => "home",
            Surface::App => "app",
            Surface::Guest => "guest",
        }
    }
}

#[derive(Deserialize)]
struct PlaceHeader {
    agent: String,
    surface: String,
}

struct PlaceFile {
    header: PlaceHeader,
    body: String,
}

fn parse_place(text: &str) -> Option<PlaceFile> {
    let (yaml, body) = crate::extensions::frontmatter::split(text);
    let header: PlaceHeader = serde_norway::from_str(yaml?).ok()?;
    let body = body.trim().to_string();
    (!body.is_empty()).then_some(PlaceFile { header, body })
}

fn place_files() -> &'static [PlaceFile] {
    static FILES: OnceLock<Vec<PlaceFile>> = OnceLock::new();
    FILES.get_or_init(|| {
        PlaceAssets::iter()
            .filter_map(|name| PlaceAssets::get(&name))
            .filter_map(|f| parse_place(&String::from_utf8_lossy(&f.data)))
            .collect()
    })
}

/// The engine's own block for `agent` on `surface`, if a place file has one.
pub(crate) fn engine_place(agent: &str, surface: Surface) -> Option<&'static str> {
    place_files()
        .iter()
        .find(|p| p.header.agent == agent && p.header.surface == surface.key())
        .map(|p| p.body.as_str())
}

/// The place text for `agent`. A guest takes the table's declaration when
/// the skill wrote one for it, else the engine's guest block; an app's own
/// agent has its SKILL.md for a place and gets none here.
pub(crate) fn place_text<'a>(
    agent: &str,
    surface: Surface,
    declared: Option<&'a Places>,
) -> Option<&'a str> {
    let skill_says = match surface {
        Surface::Guest => declared.and_then(|p| p.text_for(agent)),
        Surface::Home | Surface::App => None,
    };
    skill_says.or_else(|| engine_place(agent, surface))
}

/// The section as it sits in the system prompt.
pub(crate) fn render(text: &str) -> String {
    format!("## Where you are\n\n{}", text.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_place_file_names_its_agent_and_surface() {
        let count = PlaceAssets::iter().count();
        assert!(count > 0, "agents/places/ ships no place");
        assert_eq!(
            place_files().len(),
            count,
            "a place file has no agent/surface frontmatter or no body"
        );
    }

    #[test]
    fn the_engine_ships_a_block_for_each_surface_the_two_agents_stand_on() {
        for (agent, surface) in [
            ("ling", Surface::Home),
            ("yinyue", Surface::Home),
            ("yinyue", Surface::Guest),
        ] {
            assert!(
                engine_place(agent, surface).is_some(),
                "{agent} has no place on {surface:?}"
            );
        }
        assert!(engine_place("memory", Surface::Home).is_none());
        assert!(
            engine_place("ling", Surface::App).is_none(),
            "an app's SKILL.md is its own agent's place"
        );
    }

    #[test]
    fn a_skill_declaration_speaks_to_its_guests_only() {
        let declared: Places =
            serde_norway::from_str("ling: The world of the game.\nyinyue: At the player's side.")
                .unwrap();
        assert_eq!(
            place_text("ling", Surface::App, Some(&declared)),
            None,
            "the app's own agent reads its SKILL.md, not a place"
        );
        assert_eq!(
            place_text("yinyue", Surface::Guest, Some(&declared)),
            Some("At the player's side.")
        );
        let none: Places = serde_norway::from_str("ling: The world of the game.").unwrap();
        assert_eq!(
            place_text("yinyue", Surface::Guest, Some(&none)),
            engine_place("yinyue", Surface::Guest),
            "no declaration for her: the engine's guest block"
        );
    }
}

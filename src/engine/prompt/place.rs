//! Where the speaking agent is — the `## Where you are` block.
//!
//! An agent's soul (`agents/<id>.md`) says who it is, the same everywhere.
//! Its place says where it stands this turn: which surface, who is there,
//! what it can do. See `doc/persona-design.md`.
//!
//! Two sources, one block:
//! - **Engine surfaces** — the agent's own session, or a member's seat at a
//!   table another agent leads. An app's own agent has no engine block: the app's
//!   SKILL.md is its place. Their text is a readable file under
//!   `agents/places/`, embedded at build time; its frontmatter names the
//!   agent and the surface it is for.
//!   A file may also name `needs: <agent>`: it adds to the base block only
//!   while that agent is here (`engine::agent::met`) — so Ling's block
//!   speaks of a companion only once she has been met.
//! - **Skills** — a skill declares `place:` per agent in its SKILL.md for
//!   the members at its table besides its lead; the declared text replaces
//!   the engine's member block. The engine names no app and no agent: it
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
    /// A member at a table another agent leads: another's session, or an
    /// app chat its line lands in.
    Member,
}

impl Surface {
    fn key(self) -> &'static str {
        match self {
            Surface::Home => "home",
            Surface::App => "app",
            Surface::Member => "member",
        }
    }
}

#[derive(Deserialize)]
struct PlaceHeader {
    agent: String,
    surface: String,
    /// Another agent this block speaks of: it counts only while that agent is here.
    #[serde(default)]
    needs: Option<String>,
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
        .find(|p| {
            p.header.agent == agent && p.header.surface == surface.key() && p.header.needs.is_none()
        })
        .map(|p| p.body.as_str())
}

/// What the engine adds to that block for each agent that is here (`present`).
fn engine_additions<'a>(
    agent: &'a str,
    surface: Surface,
    present: &'a dyn Fn(&str) -> bool,
) -> impl Iterator<Item = &'static str> + 'a {
    place_files()
        .iter()
        .filter(move |p| p.header.agent == agent && p.header.surface == surface.key())
        .filter(move |p| p.header.needs.as_deref().is_some_and(present))
        .map(|p| p.body.as_str())
}

/// The place text for `agent`. A member takes the table's declaration when
/// the skill wrote one for it, else the engine's member block; an app's own
/// agent has its SKILL.md for a place and gets none here. The engine's block
/// carries what it says of each other agent only while that agent is here.
pub(crate) fn place_text(
    agent: &str,
    surface: Surface,
    declared: Option<&Places>,
    present: &dyn Fn(&str) -> bool,
) -> Option<String> {
    let skill_says = match surface {
        Surface::Member => declared.and_then(|p| p.text_for(agent)),
        Surface::Home | Surface::App => None,
    };
    if let Some(says) = skill_says {
        return Some(says.to_string());
    }
    let mut parts = vec![engine_place(agent, surface)?];
    parts.extend(engine_additions(agent, surface, present));
    Some(parts.join("\n\n"))
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
            ("yinyue", Surface::Member),
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
    fn a_skill_declaration_speaks_to_its_members_only() {
        let declared: Places =
            serde_norway::from_str("ling: The world of the game.\nyinyue: At the player's side.")
                .unwrap();
        assert_eq!(
            place_text("ling", Surface::App, Some(&declared), &|_| true),
            None,
            "the app's own agent reads its SKILL.md, not a place"
        );
        assert_eq!(
            place_text("yinyue", Surface::Member, Some(&declared), &|_| true).as_deref(),
            Some("At the player's side.")
        );
        let none: Places = serde_norway::from_str("ling: The world of the game.").unwrap();
        assert_eq!(
            place_text("yinyue", Surface::Member, Some(&none), &|_| true).as_deref(),
            engine_place("yinyue", Surface::Member),
            "no declaration for her: the engine's member block"
        );
    }

    /// Ling's block speaks of the companion only while she is here.
    #[test]
    fn a_block_speaks_of_another_agent_only_while_she_is_here() {
        let with = place_text("ling", Surface::Home, None, &|a| a == "yinyue").unwrap();
        let without = place_text("ling", Surface::Home, None, &|_| false).unwrap();
        assert!(with.contains("Yinyue lives on this Mac"));
        assert!(
            !without.contains("Yinyue"),
            "unmet: Ling never speaks of her"
        );
        assert!(with.starts_with(&without), "the base is shared");
        assert_eq!(engine_place("ling", Surface::Home), Some(without.as_str()));
    }
}

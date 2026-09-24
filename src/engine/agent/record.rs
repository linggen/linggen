//! Engine's runtime record for an agent definition.
//!
//! Parsed by `extensions::agents` from `agents/*.md` frontmatter;
//! consumed by the engine for spawn, system-prompt assembly, tool
//! gating, and model routing.
//!
//! The struct lives in `engine/` because every field is something
//! the engine reads or routes against. Disk-level concerns (the
//! `---` frontmatter splitter, file discovery, global+project
//! layering) stay in `extensions::agents`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentSpec {
    pub name: String,
    pub description: String,
    pub tools: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub personality: Option<String>,
    /// Per-agent reasoning-effort override ("low"|"medium"|"high"). Applied to
    /// reasoning-capable models (gpt-5.x / o-series / deepseek-r) at call time,
    /// overriding the model config's default — lets a conversational agent like
    /// Yinyue run the flagship at low effort for snappier replies.
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    /// Other names a person may address the agent by at the start of a
    /// message (`@银月 …`), beside its id. Matched case-insensitively.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    /// Not addressable by a person: kept out of the chat's `@` / `@@` lists
    /// and never the target of a leading `@name`. The engine still runs it
    /// (missions, delegation).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub internal: bool,
}

impl AgentSpecFile {
    /// Whether `name` (an `@name` at a message's start) addresses this agent:
    /// its id or one of its declared aliases, case-insensitively.
    /// An internal agent answers to no one.
    pub fn answers_to(&self, name: &str) -> bool {
        let name = name.trim();
        !self.spec.internal
            && !name.is_empty()
            && (self.agent_id.eq_ignore_ascii_case(name)
                || self.spec.name.eq_ignore_ascii_case(name)
                || self
                    .spec
                    .aliases
                    .iter()
                    .any(|a| a.trim().to_lowercase() == name.to_lowercase()))
    }
}

/// A loaded agent: frontmatter (`spec`) + body (`system_prompt`) +
/// the file it came from. Produced by `extensions::agents`; consumed
/// by the engine for spawn, system-prompt assembly, and the admin
/// UI's agent listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpecFile {
    pub agent_id: String,
    pub spec: AgentSpec,
    pub spec_path: PathBuf,
    #[serde(skip)]
    pub system_prompt: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str, aliases: &[&str]) -> AgentSpecFile {
        AgentSpecFile {
            agent_id: id.into(),
            spec: AgentSpec {
                name: id.into(),
                description: String::new(),
                tools: Vec::new(),
                model: None,
                personality: None,
                reasoning_effort: None,
                aliases: aliases.iter().map(|a| a.to_string()).collect(),
                internal: false,
            },
            spec_path: PathBuf::new(),
            system_prompt: String::new(),
        }
    }

    #[test]
    fn an_agent_answers_to_its_id_and_its_declared_aliases() {
        let yinyue = spec("yinyue", &["银月"]);
        assert!(yinyue.answers_to("Yinyue"));
        assert!(yinyue.answers_to("银月"));
        assert!(!yinyue.answers_to("ling"));
        assert!(!yinyue.answers_to(""));
        assert!(!spec("ling", &[]).answers_to("银月"));
    }

    #[test]
    fn an_internal_agent_answers_to_no_one() {
        let mut memory = spec("memory", &["记忆"]);
        memory.spec.internal = true;
        assert!(!memory.answers_to("memory"));
        assert!(!memory.answers_to("记忆"));
    }

    #[test]
    fn internal_is_read_from_frontmatter_and_defaults_off() {
        let on: AgentSpec =
            serde_yml::from_str("name: memory\ndescription: d\ntools: []\ninternal: true\n")
                .unwrap();
        assert!(on.internal);
        let off: AgentSpec =
            serde_yml::from_str("name: ling\ndescription: d\ntools: []\n").unwrap();
        assert!(!off.internal);
    }
}

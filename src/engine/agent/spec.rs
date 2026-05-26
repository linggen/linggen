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

//! In-memory representation of a skill — the engine's runtime record
//! for an extension bundle. Parsed by `extensions::skills` from
//! `SKILL.md` on disk; consumed by the engine for session activation,
//! permission grants, tool registration, and capability dispatch.
//!
//! The struct lives in `engine/` because every field is something the
//! engine reads or routes against. Disk-level concerns (frontmatter
//! splitter, file discovery, marketplace fetch) stay in `extensions/`.

use crate::engine::permission::Grants as SkillPermission;
use crate::engine::skill_tool::SkillToolDef;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    /// How to launch: "web" (serve static files), "bash" (run script), "url" (open URL).
    pub launcher: String,
    /// Entry point: filename (web/bash) or URL (url launcher).
    pub entry: String,
    /// Suggested panel width in pixels.
    #[serde(default)]
    pub width: Option<u32>,
    /// Suggested panel height in pixels.
    #[serde(default)]
    pub height: Option<u32>,
}

/// A skill's binding for a capability it claims to `provide`. The
/// engine owns the capability's tool names / schemas / tiers (see
/// `engine::capabilities`); this struct tells the engine *where* on the
/// skill's daemon each tool is served, how to autostart it, and where
/// to probe for health.
///
/// One `CapabilityImpl` per capability in the skill's `implements:`
/// block. Example (`skills/ling-mem/SKILL.md`):
///
/// ```yaml
/// provides: [memory]
/// implements:
///   memory:
///     base_url: http://127.0.0.1:9888
///     autostart: "ling-mem start"
///     healthcheck: /api/health
///     tools:
///       Memory_query.get:    /api/memory/get
///       Memory_query.search: /api/memory/search
///       Memory_query.list:   /api/memory/list
///       Memory_write.add:    /api/memory/add
///       Memory_write.update: /api/memory/update
///       Memory_write.delete: /api/memory/delete
/// ```
///
/// Verb-dispatched tools (`Memory_query` / `Memory_write`) key entries
/// as `<tool>.<verb>` — the engine reads the verb from the call args,
/// looks up the corresponding URL, strips the verb, and POSTs.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CapabilityImpl {
    /// Root of the skill's HTTP surface. Engine concatenates this with
    /// the tool's path to form the dispatch URL. Include the scheme,
    /// host, and port (e.g. `http://127.0.0.1:9888`).
    pub base_url: String,
    /// Command the engine runs when the daemon isn't reachable on the
    /// first call. Parsed with shell-split semantics. The first token
    /// is resolved against `$SKILL_DIR/bin/` first, then `$PATH`.
    #[serde(default)]
    pub autostart: Option<String>,
    /// Path that returns 200 when the daemon is healthy. Reserved for
    /// the engine's future liveness probe; not consumed today.
    #[serde(default = "default_capability_healthcheck")]
    pub healthcheck: String,
    /// Map from capability tool name → path on the daemon. For
    /// verb-dispatched tools (e.g. `Memory_query`, `Memory_write`),
    /// keys are `<tool>.<verb>` (e.g. `Memory_query.search`).
    #[serde(default)]
    pub tools: HashMap<String, String>,
}

fn default_capability_healthcheck() -> String {
    "/api/health".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum SkillSource {
    Global,
    Project,
    Compat { label: String },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
    pub source: SkillSource,
    #[serde(default)]
    pub tool_defs: Vec<SkillToolDef>,
    #[serde(default)]
    pub argument_hint: Option<String>,
    #[serde(default)]
    pub disable_model_invocation: bool,
    #[serde(default = "default_user_invocable")]
    pub user_invocable: bool,
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    /// Skills that the agent may invoke via the `Skill` tool while this skill
    /// is active in a session. Mirrors the mission-frontmatter field of the
    /// same name. Semantics:
    /// - omitted → default to `[<this skill's name>]` (only the skill itself)
    /// - `["*"]` → no whitelist (any installed skill reachable)
    /// - `["a", "b", …]` → exactly those skills (the active skill is
    ///   automatically included)
    #[serde(default)]
    pub allow_skills: Option<Vec<String>>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub trigger: Option<String>,
    #[serde(default)]
    pub app: Option<AppConfig>,
    /// Permission request — if set, user is prompted to approve before skill runs.
    #[serde(default)]
    pub permission: Option<SkillPermission>,
    /// Optional starting cwd for sessions invoking this skill. Tilde-expandable
    /// (`~/.linggen`). When omitted, sessions inherit the user's `home_path`.
    /// Mirrors the `cwd:` field in mission frontmatter.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Install script path relative to skill directory. Run once on install.
    #[serde(default)]
    pub install: Option<String>,
    /// Capabilities this skill implements (e.g. `["memory"]`). Linggen core
    /// routes built-in tool families (e.g. `Memory.*`) to whichever installed
    /// skill provides the capability. See `doc/skill-spec.md` and
    /// `doc/memory-spec.md`.
    #[serde(default)]
    pub provides: Option<Vec<String>>,
    /// Bindings for capabilities the skill `provides:`. Keyed by the
    /// capability's name (`"memory"`). One entry per capability this
    /// skill implements. The engine owns each capability's canonical tool
    /// contract (names, schemas, tiers — see `engine::capabilities`); this
    /// map tells the engine *where* on the skill's daemon each tool is
    /// served. Absent on skills that don't implement any capability.
    #[serde(default)]
    pub implements: Option<HashMap<String, CapabilityImpl>>,
    /// Filesystem path to the skill directory (set at load time, not serialized to clients).
    #[serde(skip)]
    pub skill_dir: Option<PathBuf>,
}

fn default_user_invocable() -> bool {
    true
}

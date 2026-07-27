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
    /// Whether the launcher offers this app as a tab. Default true — declaring
    /// `list: false` keeps a skill that is installed but not finished out of
    /// the way without uninstalling it, so it still runs when opened directly.
    #[serde(default = "listed_by_default")]
    pub list: bool,
}

fn listed_by_default() -> bool {
    true
}

/// One sidecar file resolved from a primary item's stem — a lyric file next to
/// a song, a cover next to a photo. `subdir` names an entry in
/// [`SyncConfig::subdirs`]; `suffix` is inserted between the stem and the
/// extension (`"聽海"` + `" (Karaoke)"` + `.mp3`).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SyncCompanion {
    pub name: String,
    pub exts: Vec<String>,
    #[serde(default)]
    pub subdir: Option<String>,
    #[serde(default)]
    pub suffix: Option<String>,
}

/// A directory the skill asks the engine to serve to paired devices.
///
/// The engine reads this and does something entirely generic with it — it
/// never learns what the files mean. See `doc/skill-spec.md` § Device sync for
/// why this is declarative rather than a module per app.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SyncConfig {
    /// Directory to serve. Tilde-expandable (`~/Music/DJ`).
    pub dir: String,
    /// Extensions that count as a primary item. Everything else in `dir` is
    /// only reachable as a companion.
    pub items: Vec<String>,
    /// Named subdirectories the skill may serve from, e.g. `karaoke: .karaoke`.
    /// A request may only name a key here — never a path.
    #[serde(default)]
    pub subdirs: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub companions: Vec<SyncCompanion>,
    /// When set, changes under `dir` are announced as `<topic>/library-changed`
    /// so devices sync on push instead of polling.
    #[serde(default)]
    pub topic: Option<String>,
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
    /// Memory namespace for this skill. When set, the engine FORCES every
    /// `Memory_query`/`Memory_write` from a session bound to this skill to be
    /// scoped to this `contexts` tag — so a focused app (e.g. CFO ↔ "cfo")
    /// only ever sees/writes its own memory, never the shared cross-app store.
    /// Omitted → the skill's memory (if it has the tools) is unscoped.
    #[serde(default)]
    pub memory_context: Option<String>,
    /// When `memory_context` is set, the engine also auto-recalls that
    /// namespace into each turn. These tune that scoped recall (per-app):
    /// cosine floor and how many rows to inject. Omitted → engine defaults
    /// (0.6 floor, 3 rows). Ignored when `memory_context` is absent.
    #[serde(default)]
    pub memory_recall_min_score: Option<f32>,
    #[serde(default)]
    pub memory_recall_count: Option<usize>,
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
    /// Directory this skill wants served to paired devices. The engine wires a
    /// generic file-sync surface from it — see `SyncConfig`.
    #[serde(default)]
    pub sync: Option<SyncConfig>,
    /// Filesystem path to the skill directory (set at load time, not serialized to clients).
    #[serde(skip)]
    pub skill_dir: Option<PathBuf>,
}

fn default_user_invocable() -> bool {
    true
}

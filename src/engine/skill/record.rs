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

/// What a skill keeps on linggen.dev for the signed-in account. Declaring it
/// makes the skill need an account: a session bound to it refuses a turn when
/// signed out. The engine does all the talking — a skill never calls out.
/// See `doc/skill-spec.md` § Cloud.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CloudConfig {
    /// What under the skill directory is kept in step across the account's
    /// devices: pulled at a turn's edges, pushed after a call that changed
    /// it. One file (`save: data/state.json`, stored as its text), or a list
    /// of files and folders stored together as one bundle.
    #[serde(default)]
    pub save: Option<SavePaths>,
    /// Names under the save never kept in the cloud — a file or folder with
    /// one of these names, at any depth, is neither pushed, pulled nor
    /// removed by a pull (`skip: [art]`: pictures stay on the device).
    #[serde(default)]
    pub skip: Vec<String>,
    /// A rolling token window on linggen.dev, named by the site. Each turn's
    /// tokens are reported to it; a spent window refuses the next turn.
    #[serde(default)]
    pub meter: Option<String>,
}

/// `cloud.save`: one path, or several kept as one bundle.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum SavePaths {
    One(String),
    Many(Vec<String>),
}

impl SavePaths {
    pub fn paths(&self) -> Vec<&str> {
        match self {
            SavePaths::One(p) => vec![p.as_str()],
            SavePaths::Many(ps) => ps.iter().map(String::as_str).collect(),
        }
    }

    /// The single-file form, whose cloud copy is the file's own text.
    pub fn is_one(&self) -> bool {
        matches!(self, SavePaths::One(_))
    }
}

impl From<&str> for SavePaths {
    fn from(p: &str) -> Self {
        SavePaths::One(p.to_string())
    }
}

/// How a skill's quests take facts the engine hears about — a paired phone
/// saying "this kind of thing happened, at this time". The engine never
/// learns what a kind means: it looks the kind up here and runs the skill's
/// own quest writer. See `doc/skill-spec.md` § Quests.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
pub struct QuestsConfig {
    /// The skill's one writer of its quest file, run in the skill's working
    /// folder: `{id}` and `{at}` are filled with a validated quest id and a
    /// UTC time (`2026-09-24T14:03:11Z`). It must never move `done_at` back,
    /// and exits 0 once the file holds the stamp (or a later one).
    pub stamp: String,
    /// Fact kind → the quest id it stamps (`photos-clean: shifu-clear`).
    #[serde(default)]
    pub facts: std::collections::BTreeMap<String, String>,
}

/// Where the skill's sessions stand for each agent: the text the engine puts
/// under `## Where you are` when that agent speaks there, and whether the
/// agent is there at all yet. Keyed by agent id. The engine names no app and
/// no agent here: it reads whatever the skill declares for whoever speaks.
/// See `doc/skill-spec.md` § Place.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(transparent)]
pub struct Places(pub std::collections::BTreeMap<String, PlaceEntry>);

/// One agent's place in a skill: plain text, or text with a presence gate.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum PlaceEntry {
    Text(String),
    Gated {
        #[serde(default)]
        text: Option<String>,
        /// The agent is absent from the skill's sessions until this state
        /// the skill keeps is set.
        #[serde(default)]
        absent_until: Option<StateFlag>,
    },
}

/// A value the skill keeps in one of its own JSON files: `path` is a
/// dot-separated key path inside `file` (relative to the skill folder).
/// Set = present and not `null`, `false`, `0`, `""`, `[]` or `{}`.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct StateFlag {
    pub file: String,
    pub path: String,
}

impl Places {
    /// The place text the skill declares for `agent`, if any.
    pub fn text_for(&self, agent: &str) -> Option<&str> {
        let text = match self.0.get(agent)? {
            PlaceEntry::Text(t) => Some(t.as_str()),
            PlaceEntry::Gated { text, .. } => text.as_deref(),
        }?;
        Some(text.trim()).filter(|t| !t.is_empty())
    }

    /// Whether `agent` is absent from this skill's sessions right now: it
    /// declares `absent_until` and the flag in `skill_dir` is not set. A
    /// file that can't be read or parsed leaves the flag unset.
    pub fn is_absent(&self, agent: &str, skill_dir: &std::path::Path) -> bool {
        self.gate_for(agent)
            .is_some_and(|flag| !flag.is_set(skill_dir))
    }

    /// The state `agent` is absent until, when the skill declares one.
    pub fn gate_for(&self, agent: &str) -> Option<&StateFlag> {
        match self.0.get(agent)? {
            PlaceEntry::Gated { absent_until, .. } => absent_until.as_ref(),
            PlaceEntry::Text(_) => None,
        }
    }
}

impl StateFlag {
    /// Read the flag from the skill's own folder. A file path that is
    /// absolute or climbs out of the folder is never read.
    pub fn is_set(&self, skill_dir: &std::path::Path) -> bool {
        let rel = std::path::Path::new(&self.file);
        let inside = rel
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)));
        if !inside {
            return false;
        }
        std::fs::read_to_string(skill_dir.join(rel))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .is_some_and(|json| value_is_set(value_at(&json, &self.path)))
    }
}

/// The value at a dot-separated key path.
fn value_at<'a>(json: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    path.split('.')
        .filter(|k| !k.is_empty())
        .try_fold(json, |v, key| v.get(key))
}

fn value_is_set(value: Option<&serde_json::Value>) -> bool {
    use serde_json::Value;
    match value {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
        Some(Value::Bool(true)) => true,
    }
}

/// How a message sent while the skill's session is mid-turn meets that turn.
/// See `doc/skill-spec.md` § Queue.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum QueueMode {
    /// The message steers: the running turn stops at its next step and the
    /// message is taken up (ordinary chat).
    #[default]
    Steer,
    /// The message waits for the whole turn — tools, words, an open question —
    /// and runs once the turn is over (a game whose turn must play out).
    AfterTurn,
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
    /// memory call from a session bound to this skill to be scoped to this
    /// `contexts` tag (`engine/tools/memory_mcp.rs`) — so a focused app (e.g. CFO ↔ "cfo")
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
    /// What this skill keeps on linggen.dev for the signed-in account — see
    /// `CloudConfig`.
    #[serde(default)]
    pub cloud: Option<CloudConfig>,
    /// The Linggen Cloud product this skill's turns bill to (sent as
    /// X-Linggen-App). Absent: the shared 'linggen' bucket. Declared by the
    /// skill so the engine never names an app — see `doc/skill-spec.md`
    /// § Product.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    /// Starter prompts shown as buttons above the skill's chat — see
    /// `doc/chat-spec.md` § Suggestions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<String>,
    /// The skill's tools hand each turn its closing question (`ask` in their
    /// result); when the model ends a turn without asking it, the engine asks
    /// it. See `doc/skill-spec.md` § Closing question.
    #[serde(default)]
    pub closing_ask: bool,
    /// Whether a busy-time message steers the running turn or waits for it.
    #[serde(default)]
    pub queue: QueueMode,
    /// Which phone facts stamp which of the skill's quests, and how.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quests: Option<QuestsConfig>,
    /// Where the skill's sessions stand for each agent — see [`Places`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<Places>,
    /// Filesystem path to the skill directory (set at load time, not serialized to clients).
    #[serde(skip)]
    pub skill_dir: Option<PathBuf>,
}

fn default_user_invocable() -> bool {
    true
}

#[cfg(test)]
mod place_tests {
    use super::*;

    fn places(yaml: &str) -> Places {
        serde_norway::from_str(yaml).unwrap()
    }

    #[test]
    fn a_place_is_plain_text_or_text_with_a_gate() {
        let p = places(
            "ling: \"You run the world here.\"\n\
             yinyue:\n  text: \"Beside the player.\"\n  absent_until: {file: data/state.json, path: companion.joined}\n",
        );
        assert_eq!(p.text_for("ling"), Some("You run the world here."));
        assert_eq!(p.text_for("yinyue"), Some("Beside the player."));
        assert_eq!(p.text_for("memory"), None);
    }

    #[test]
    fn absent_until_the_skill_sets_its_flag() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("data")).unwrap();
        let p =
            places("yinyue:\n  absent_until: {file: data/state.json, path: companion.joined}\n");
        let write = |json: &str| std::fs::write(dir.path().join("data/state.json"), json).unwrap();

        assert!(p.is_absent("yinyue", dir.path()), "no file yet: absent");
        write(r#"{"companion": null}"#);
        assert!(p.is_absent("yinyue", dir.path()));
        write(r#"{"companion": {"joined": "2026-09-18"}}"#);
        assert!(!p.is_absent("yinyue", dir.path()));
        assert!(!p.is_absent("ling", dir.path()), "no gate: always present");
        assert_eq!(p.text_for("yinyue"), None, "a gate alone carries no text");
    }

    #[test]
    fn a_flag_never_reads_outside_the_skill_folder() {
        let dir = tempfile::tempdir().unwrap();
        for file in ["../state.json", "/etc/hosts"] {
            let flag = StateFlag {
                file: file.into(),
                path: "x".into(),
            };
            assert!(!flag.is_set(dir.path()), "{file}");
        }
    }
}

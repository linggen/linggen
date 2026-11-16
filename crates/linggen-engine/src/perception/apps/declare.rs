//! The `world-line:` a skill declares, checked where its frontmatter is read.
//!
//! ```yaml
//! world-line:
//!   file: data/world.json
//!   fields: [songs, playing]
//! ```
//!
//! A declaration names a small JSON file inside the skill's own folder and the
//! top-level keys of it that make the app's line. Nothing else: the engine
//! never runs skill code for it, and never learns what the values mean.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Mutex;

/// Fields one app's line may carry.
pub const FIELDS_MAX: usize = 6;
/// A field key: what the skill writes as the JSON key, and the line's label.
pub const KEY_MAX: usize = 32;
/// The declared file's path, relative to the skill's folder.
pub const FILE_PATH_MAX: usize = 128;

/// Where an app's line comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldLine {
    /// A `.json` file, relative to the skill's folder.
    pub file: String,
    /// Top-level keys of that file, in the order the line shows them.
    pub fields: Vec<String>,
}

/// The declaration as written, checked. The reason names what is wrong, for
/// the one log line it earns.
pub fn check(raw: serde_json::Value) -> Result<WorldLine, String> {
    let decl: WorldLine =
        serde_json::from_value(raw).map_err(|e| format!("not {{file, fields}}: {e}"))?;
    check_file(&decl.file)?;
    check_fields(&decl.fields)?;
    Ok(decl)
}

fn check_file(file: &str) -> Result<(), String> {
    let path = std::path::Path::new(file);
    let inside = !file.is_empty()
        && file.len() <= FILE_PATH_MAX
        && path.is_relative()
        && path
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)));
    if !inside {
        return Err(format!(
            "file '{file}' must be a plain path inside the skill"
        ));
    }
    if path.extension().is_none_or(|e| e != "json") {
        return Err(format!("file '{file}' must be a .json file"));
    }
    Ok(())
}

fn check_fields(fields: &[String]) -> Result<(), String> {
    if fields.is_empty() || fields.len() > FIELDS_MAX {
        return Err(format!(
            "{} fields; a line takes 1 to {FIELDS_MAX}",
            fields.len()
        ));
    }
    if let Some(bad) = fields.iter().find(|f| !is_key(f)) {
        return Err(format!("field '{bad}' is not a key of [a-z0-9_]"));
    }
    let unique: HashSet<&String> = fields.iter().collect();
    if unique.len() != fields.len() {
        return Err("a field is named twice".to_string());
    }
    Ok(())
}

fn is_key(k: &str) -> bool {
    (1..=KEY_MAX).contains(&k.len())
        && k.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// What a skill's frontmatter keeps of its `world-line:`. Only an installed
/// skill's counts: a project's copy would otherwise write into every prompt
/// on the machine. Anything else is logged once and dropped; the skill still
/// loads.
pub fn usable(raw: Option<serde_json::Value>, skill: &str, installed: bool) -> Option<WorldLine> {
    let raw = raw?;
    if !installed {
        warn_once(skill, "world-line is for installed skills only; ignored");
        return None;
    }
    check(raw)
        .map_err(|why| warn_once(skill, &format!("world-line ignored: {why}")))
        .ok()
}

/// How many distinct warnings are kept; past it, new ones go unsaid.
const WARNED_MAX: usize = 256;

/// Each problem is named once per run. A skill is re-read on every
/// activation and its file on every turn, and the same warning a hundred
/// times buries the log it is in.
pub(crate) fn warn_once(skill: &str, what: &str) {
    static SEEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let key = format!("{skill}: {what}");
    let mut g = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    let seen = g.get_or_insert_with(HashSet::new);
    if seen.len() < WARNED_MAX && seen.insert(key) {
        tracing::warn!("skill '{skill}': {what}");
    }
}

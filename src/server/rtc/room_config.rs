//! Local room configuration — which models to share with proxy room consumers.
//!
//! Stored at `~/.linggen/room_config.toml`.
//! This is separate from the room metadata on linggen.dev (which stores name, type, etc.)
//! because the model list is local to each instance.

use serde::{Deserialize, Serialize};

/// Default allowed tools for consumers — safe, no filesystem access.
fn default_allowed_tools() -> Vec<String> {
    vec![
        "WebSearch".to_string(),
        "WebFetch".to_string(),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomConfig {
    /// Model IDs that are shared with room consumers.
    /// If empty, no models are shared (safe default).
    #[serde(default)]
    pub shared_models: Vec<String>,

    /// Tools that consumers are allowed to use.
    /// Default: ["WebSearch", "WebFetch"] — safe, no filesystem access.
    /// Owner can expand for trusted consumers (e.g. add "Read", "Bash" for family).
    #[serde(default = "default_allowed_tools")]
    pub allowed_tools: Vec<String>,

    /// Skills that consumers can use (by skill name).
    /// Default: empty (no skills). Owner adds specific skills.
    #[serde(default)]
    pub allowed_skills: Vec<String>,
}

impl Default for RoomConfig {
    fn default() -> Self {
        Self {
            shared_models: Vec::new(),
            allowed_tools: default_allowed_tools(),
            allowed_skills: Vec::new(),
        }
    }
}

fn room_config_path() -> std::path::PathBuf {
    crate::paths::linggen_home().join("room_config.toml")
}

pub fn load_room_config() -> RoomConfig {
    let path = room_config_path();
    if !path.exists() {
        return RoomConfig::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => toml::from_str(&content).unwrap_or_default(),
        Err(_) => RoomConfig::default(),
    }
}

/// Derive UserPermission from the allowed tools list.
/// Maps the tool preset pattern to a permission level.
pub fn tools_to_permission(tools: &[String]) -> super::UserPermission {
    let has_write = tools.iter().any(|t| matches!(t.as_str(), "Write" | "Edit" | "Bash"));
    let has_read = tools.iter().any(|t| matches!(t.as_str(), "WebSearch" | "WebFetch" | "Read" | "Glob" | "Grep"));
    if has_write { super::UserPermission::Edit }
    else if has_read { super::UserPermission::Read }
    else if tools.is_empty() { super::UserPermission::Chat }
    else { super::UserPermission::Read }
}

pub fn save_room_config(config: &RoomConfig) -> anyhow::Result<()> {
    let path = room_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let toml_str = toml::to_string_pretty(config)?;
    std::fs::write(&path, toml_str)?;
    Ok(())
}

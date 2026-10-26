//! Small per-session files beside `messages.jsonl`. Every write replaces a
//! whole inode, so a crash never leaves a half-written resume pointer or
//! permission grant behind.
use super::install::VERSION;
use anyhow::Result;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const CHECKPOINT: &str = "claude.json";
const CAPABILITIES: &str = "claude-capabilities.json";
const AUTO_VERIFIED: &str = "claude-auto-ok";

pub(super) fn checkpoint_path(dir: &Path) -> PathBuf {
    dir.join(CHECKPOINT)
}

pub(super) fn permissions_path(dir: &Path) -> PathBuf {
    dir.join("claude-permissions.json")
}

pub(super) fn read(path: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

pub(super) fn write(path: &Path, value: &Value) -> Result<()> {
    let staged = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&staged, serde_json::to_vec(value)?)?;
    std::fs::rename(staged, path)?;
    Ok(())
}

/// Read-modify-write of the resume pointer; a missing file starts empty.
pub(super) fn update_checkpoint(dir: &Path, change: impl FnOnce(&mut Value)) -> Result<()> {
    let path = checkpoint_path(dir);
    let mut checkpoint = read(&path).unwrap_or_else(|| json!({}));
    change(&mut checkpoint);
    write(&path, &checkpoint)
}

/// Stamp the command list with the SDK version that produced it.
pub(super) fn set_commands(checkpoint: &mut Value, commands: &Value) {
    checkpoint["commands"] = commands.clone();
    checkpoint["commands_version"] = json!(VERSION);
}

/// The cached slash commands, unless an SDK upgrade made them stale.
pub(super) fn cached_commands(dir: &Path) -> Option<Value> {
    [CHECKPOINT, CAPABILITIES]
        .iter()
        .filter_map(|name| read(&dir.join(name)))
        .find(|value| value["commands"].is_array() && value["commands_version"] == VERSION)
        .map(|value| json!({"commands": value["commands"]}))
}

pub(super) fn save_capabilities(dir: &Path, commands: &Value) -> Result<()> {
    write(
        &dir.join(CAPABILITIES),
        &json!({"commands": commands, "commands_version": VERSION}),
    )
}

/// Auto mode depends on the account and model, so it is only offered inline
/// once this session has seen the SDK accept it.
pub(super) fn mark_auto_verified(dir: &Path) {
    let _ = std::fs::write(dir.join(AUTO_VERIFIED), VERSION);
}

pub(super) fn auto_verified(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join(AUTO_VERIFIED)).is_ok_and(|v| v == VERSION)
}

/// Resume only the conversation that ran in this folder.
pub(super) fn resume_id(dir: &Path, cwd: &str) -> Option<String> {
    let checkpoint = read(&checkpoint_path(dir))?;
    if checkpoint["cwd"].as_str().is_some_and(|saved| saved != cwd) {
        return None;
    }
    checkpoint["session_id"].as_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_checkpoint_starts_empty_and_stays_atomic() {
        let dir = tempfile::tempdir().unwrap();
        update_checkpoint(dir.path(), |c| set_commands(c, &json!([{"name":"clear"}]))).unwrap();
        assert_eq!(
            cached_commands(dir.path()).unwrap()["commands"][0]["name"],
            "clear"
        );
        let leftovers = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(leftovers, 1);
    }

    #[test]
    fn commands_from_another_sdk_version_are_stale() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &checkpoint_path(dir.path()),
            &json!({"commands":[], "commands_version":"0.0.1"}),
        )
        .unwrap();
        assert!(cached_commands(dir.path()).is_none());
    }

    #[test]
    fn resume_requires_the_same_folder() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &checkpoint_path(dir.path()),
            &json!({"session_id":"abc", "cwd":"/a"}),
        )
        .unwrap();
        assert_eq!(resume_id(dir.path(), "/a").as_deref(), Some("abc"));
        assert_eq!(resume_id(dir.path(), "/b"), None);
    }

    #[test]
    fn auto_is_offered_only_after_the_sdk_accepted_it() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!auto_verified(dir.path()));
        mark_auto_verified(dir.path());
        assert!(auto_verified(dir.path()));
    }
}

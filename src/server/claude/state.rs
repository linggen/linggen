//! Small per-session files beside `messages.jsonl`. Every write replaces a
//! whole inode, so a crash never leaves a half-written resume pointer or
//! command cache behind.
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

/// The conversation to resume and the folder it started in. The SDK keeps a
/// conversation under the folder it started in, so a resumed runner starts
/// there again. Only the conversation of this folder resumes: the one that
/// started here, or the one whose shell last moved here (`followed`).
pub(super) fn resume(dir: &Path, cwd: &str) -> Option<(String, String)> {
    let checkpoint = read(&checkpoint_path(dir))?;
    let id = checkpoint["session_id"].as_str()?.to_owned();
    let started = checkpoint["cwd"].as_str().unwrap_or(cwd).to_owned();
    let followed = checkpoint["followed"].as_str();
    (started == cwd || followed == Some(cwd)).then_some((id, started))
}

/// The folder a runner for a session now in `cwd` starts in.
pub(super) fn launch_folder(dir: &Path, cwd: &str) -> String {
    resume(dir, cwd).map_or_else(|| cwd.to_owned(), |(_, started)| started)
}

/// The SDK's own copy of the session's conversation under `sdk_state`
/// (`CLAUDE_CONFIG_DIR`): its transcript and its subagents' folder, found by
/// the checkpoint's conversation id. Nothing when the id is not a UUID.
pub(super) fn transcript_paths(dir: &Path, sdk_state: &Path) -> Vec<PathBuf> {
    let Some(checkpoint) = read(&checkpoint_path(dir)) else {
        return Vec::new();
    };
    let Some(id) = checkpoint["session_id"].as_str().filter(|id| uuid::Uuid::parse_str(id).is_ok())
    else {
        return Vec::new();
    };
    let projects = sdk_state.join("projects");
    let encoded = checkpoint["cwd"].as_str().map(encode_folder);
    let folder = encoded
        .map(|name| projects.join(name))
        .filter(|folder| folder.join(format!("{id}.jsonl")).exists())
        .or_else(|| folder_holding(&projects, id));
    folder.map_or_else(Vec::new, |folder| {
        vec![folder.join(format!("{id}.jsonl")), folder.join(id)]
    })
}

/// Remove the SDK's copy of the session's conversation ([`transcript_paths`]).
pub(super) fn forget_transcript(dir: &Path, sdk_state: &Path) {
    for path in transcript_paths(dir, sdk_state) {
        let _ = match path.is_dir() {
            true => std::fs::remove_dir_all(&path),
            false => std::fs::remove_file(&path),
        };
    }
}

/// The SDK names a project folder after its path, every character but a
/// letter or digit a dash.
fn encode_folder(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The project folder holding `<id>.jsonl`, for a path the SDK shortened.
fn folder_holding(projects: &Path, id: &str) -> Option<PathBuf> {
    std::fs::read_dir(projects)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|folder| folder.join(format!("{id}.jsonl")).is_file())
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
    fn resume_requires_the_folder_it_started_in_or_last_moved_to() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &checkpoint_path(dir.path()),
            &json!({"session_id":"abc", "cwd":"/a", "followed":"/a/src"}),
        )
        .unwrap();
        let started = Some(("abc".to_string(), "/a".to_string()));
        assert_eq!(resume(dir.path(), "/a"), started);
        assert_eq!(resume(dir.path(), "/a/src"), started);
        assert_eq!(launch_folder(dir.path(), "/a/src"), "/a");
        assert_eq!(resume(dir.path(), "/b"), None);
        assert_eq!(launch_folder(dir.path(), "/b"), "/b");
    }

    #[test]
    fn only_the_sessions_own_transcript_is_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let sdk = tempfile::tempdir().unwrap();
        let id = "6496888b-e6e3-40d4-8494-dc4fc8025ea5";
        let project = sdk.path().join("projects/-work-my-repo");
        std::fs::create_dir_all(project.join(id).join("subagents")).unwrap();
        std::fs::write(project.join(format!("{id}.jsonl")), "{}").unwrap();
        std::fs::write(project.join("other.jsonl"), "{}").unwrap();
        write(&checkpoint_path(dir.path()), &json!({"session_id":id, "cwd":"/work/my.repo"})).unwrap();
        forget_transcript(dir.path(), sdk.path());
        assert!(!project.join(format!("{id}.jsonl")).exists());
        assert!(!project.join(id).exists());
        assert!(project.join("other.jsonl").exists(), "another conversation was touched");

        // An id that is not a UUID names nothing, however it is spelled.
        std::fs::write(project.join("x.jsonl"), "{}").unwrap();
        write(&checkpoint_path(dir.path()), &json!({"session_id":"../x", "cwd":"/work/my.repo"})).unwrap();
        assert!(transcript_paths(dir.path(), sdk.path()).is_empty());
    }

    #[test]
    fn a_transcript_under_a_shortened_folder_name_is_still_found() {
        let dir = tempfile::tempdir().unwrap();
        let sdk = tempfile::tempdir().unwrap();
        let id = "19eed1ef-c68f-4e09-af6c-dd5becc3e1b6";
        let project = sdk.path().join("projects/-very-long-abc123");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join(format!("{id}.jsonl")), "{}").unwrap();
        write(&checkpoint_path(dir.path()), &json!({"session_id":id, "cwd":"/very/long/path"})).unwrap();
        forget_transcript(dir.path(), sdk.path());
        assert!(!project.join(format!("{id}.jsonl")).exists());
    }

    #[test]
    fn auto_is_offered_only_after_the_sdk_accepted_it() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!auto_verified(dir.path()));
        mark_auto_verified(dir.path());
        assert!(auto_verified(dir.path()));
    }
}

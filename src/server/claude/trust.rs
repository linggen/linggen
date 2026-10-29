//! Folder trust, the way Claude Code asks it: a folder's own Claude
//! configuration (`.claude/settings*.json` hooks, env and apiKeyHelper,
//! CLAUDE.md, skills, `.mcp.json`) loads only after the owner trusted that
//! folder once. Trusting a folder trusts what is inside it. A folder with no
//! Claude configuration of its own is never asked about: there is nothing to
//! trust, and it runs on user settings until some appears. A "no" holds for
//! that Claude session and folder only — kept in the session's directory so
//! an engine restart does not ask again — and a later session asks again.
use super::interaction::Interaction;
use crate::engine::tools::{AskUserOption, AskUserQuestion};
use anyhow::Result;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const TRUST: &str = "Yes, trust this folder";
const DECLINE: &str = "No, run without its settings";
/// In a session's directory: the folder that session was told not to trust.
const DECLINED: &str = "claude-trust-declined.json";
/// What a folder brings to a Claude session when its settings load.
const OWN_CONFIG: [&str; 4] = [".claude", "CLAUDE.md", "CLAUDE.local.md", ".mcp.json"];

pub(super) fn trust_file() -> PathBuf {
    crate::paths::linggen_home().join("runtimes/claude-agent-sdk/trusted-folders.json")
}

fn trusted_folders(file: &Path) -> Vec<PathBuf> {
    std::fs::read(file)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn canonical(folder: &Path) -> PathBuf {
    folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf())
}

pub(super) fn is_trusted_in(file: &Path, folder: &Path) -> bool {
    let folder = canonical(folder);
    trusted_folders(file)
        .iter()
        .any(|trusted| folder.starts_with(trusted))
}

pub(super) fn remember_in(file: &Path, folder: &Path) -> Result<()> {
    let mut folders = trusted_folders(file);
    let folder = canonical(folder);
    if !folders.contains(&folder) {
        folders.push(folder);
    }
    crate::state_fs::devices::write_atomic(file, &serde_json::to_vec_pretty(&folders)?)?;
    Ok(())
}

/// Whether the folder, or a folder above it short of the home folder (whose
/// `.claude` is the user's own settings), carries Claude configuration.
pub(super) fn has_own_config(folder: &Path) -> bool {
    let home = dirs::home_dir().map(|home| canonical(&home));
    canonical(folder)
        .ancestors()
        .take_while(|dir| Some(*dir) != home.as_deref())
        .any(|dir| OWN_CONFIG.iter().any(|name| dir.join(name).exists()))
}

pub(super) fn declined_in(session_dir: &Path, folder: &Path) -> bool {
    std::fs::read(session_dir.join(DECLINED))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<PathBuf>(&bytes).ok())
        .is_some_and(|declined| declined == canonical(folder))
}

fn decline_in(session_dir: &Path, folder: &Path) -> Result<()> {
    crate::state_fs::devices::write_atomic(
        &session_dir.join(DECLINED),
        &serde_json::to_vec(&canonical(folder))?,
    )?;
    Ok(())
}

/// The session's conversation ended (Clear, SDK reset): a new one asks again.
pub(crate) fn forget_decline(session_dir: &Path) {
    let _ = std::fs::remove_file(session_dir.join(DECLINED));
}

/// The SDK setting sources a folder may contribute.
pub(super) fn setting_sources(trusted: bool) -> Value {
    if trusted {
        json!(["user", "project", "local"])
    } else {
        json!(["user"])
    }
}

/// Trust known without asking: probes never prompt, so an unknown folder
/// probes with user settings only.
pub(super) fn known(state: &crate::server::ServerState, session: &str, cwd: &str) -> bool {
    let dir = state.manager.global_sessions.session_dir(session);
    !declined_in(&dir, Path::new(cwd)) && is_trusted_in(&trust_file(), Path::new(cwd))
}

impl Interaction {
    /// Ask once per folder before its own configuration can run.
    pub async fn folder_trust(&self) -> Result<bool> {
        let cwd = Path::new(&self.meta.cwd);
        if is_trusted_in(&trust_file(), cwd) {
            return Ok(true);
        }
        let dir = self.state.manager.global_sessions.session_dir(&self.meta.id);
        if !has_own_config(cwd) || declined_in(&dir, cwd) {
            return Ok(false);
        }
        let question = AskUserQuestion {
            question: format!(
                "Trust {}? Claude will load this folder's own settings, hooks and CLAUDE.md.",
                self.meta.cwd
            ),
            header: "Folder trust".into(),
            multi_select: false,
            options: [TRUST, DECLINE]
                .into_iter()
                .map(|label| AskUserOption {
                    label: label.into(),
                    description: None,
                    preview: None,
                })
                .collect(),
        };
        let answers = self.ask(vec![question], self.question_id.clone()).await?;
        let trusted = answers
            .first()
            .and_then(|answer| answer.selected.first())
            .is_some_and(|label| label == TRUST);
        if trusted {
            remember_in(&trust_file(), cwd)?;
        } else {
            decline_in(&dir, cwd)?;
        }
        Ok(trusted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trusted_folder_trusts_its_children_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("trust.json");
        let project = dir.path().join("project");
        let child = project.join("crate");
        let sibling = dir.path().join("other");
        for folder in [&child, &sibling] {
            std::fs::create_dir_all(folder).unwrap();
        }
        assert!(!is_trusted_in(&file, &project));
        remember_in(&file, &project).unwrap();
        remember_in(&file, &project).unwrap();
        assert!(is_trusted_in(&file, &project));
        assert!(is_trusted_in(&file, &child));
        assert!(!is_trusted_in(&file, &sibling));
        assert_eq!(trusted_folders(&file).len(), 1);
    }

    #[test]
    fn only_a_folder_with_claude_configuration_is_asked_about() {
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("bare");
        let project = dir.path().join("project");
        let nested = project.join("crate/src");
        for folder in [&bare, &nested] {
            std::fs::create_dir_all(folder).unwrap();
        }
        assert!(!has_own_config(&bare));
        std::fs::write(project.join("CLAUDE.md"), "rules").unwrap();
        assert!(has_own_config(&project));
        assert!(has_own_config(&nested), "a parent's CLAUDE.md loads too");
        std::fs::create_dir_all(bare.join(".claude")).unwrap();
        assert!(has_own_config(&bare));
    }

    #[test]
    fn a_no_holds_for_that_session_and_folder_until_the_conversation_ends() {
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("session");
        let folder = dir.path().join("project");
        let other = dir.path().join("other");
        for path in [&session, &folder, &other] {
            std::fs::create_dir_all(path).unwrap();
        }
        assert!(!declined_in(&session, &folder));
        decline_in(&session, &folder).unwrap();
        assert!(declined_in(&session, &folder), "kept on disk, across a restart");
        assert!(!declined_in(&session, &other), "another folder asks again");
        forget_decline(&session);
        assert!(!declined_in(&session, &folder));
    }

    #[test]
    fn untrusted_folders_load_user_settings_only() {
        assert_eq!(setting_sources(false), json!(["user"]));
        assert_eq!(setting_sources(true), json!(["user", "project", "local"]));
    }
}

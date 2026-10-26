//! Folder trust, the way Claude Code asks it: a folder's own Claude
//! configuration (`.claude/settings*.json` hooks, env and apiKeyHelper,
//! CLAUDE.md, skills, `.mcp.json`) loads only after the owner trusted that
//! folder once. Trusting a folder trusts what is inside it. A "no" holds for
//! that Claude session only, so a later session asks again.
use super::interaction::Interaction;
use crate::engine::tools::{AskUserOption, AskUserQuestion};
use anyhow::Result;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const TRUST: &str = "Yes, trust this folder";
const DECLINE: &str = "No, run without its settings";

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
    !state.claude.declined_trust(session) && is_trusted_in(&trust_file(), Path::new(cwd))
}

impl Interaction {
    /// Ask once per folder before its own configuration can run.
    pub async fn folder_trust(&self) -> Result<bool> {
        let cwd = Path::new(&self.meta.cwd);
        if is_trusted_in(&trust_file(), cwd) {
            return Ok(true);
        }
        if self.state.claude.declined_trust(&self.meta.id) {
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
            self.state.claude.decline_trust(&self.meta.id);
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
    fn untrusted_folders_load_user_settings_only() {
        assert_eq!(setting_sources(false), json!(["user"]));
        assert_eq!(setting_sources(true), json!(["user", "project", "local"]));
    }
}

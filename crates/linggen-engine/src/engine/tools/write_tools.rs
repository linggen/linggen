use super::tool_helpers::{expand_tilde, sanitize_rel_path};
use super::{ToolResult, Tools};
use anyhow::Result;
use serde::Deserialize;
use std::path::Path;
use std::time::{Duration, SystemTime};

#[derive(PartialEq, Eq)]
pub(super) struct FileStamp {
    modified: SystemTime,
    len: u64,
}

impl FileStamp {
    pub(super) fn at(path: &Path) -> std::io::Result<Self> {
        let metadata = std::fs::metadata(path)?;
        Ok(Self {
            modified: metadata.modified()?,
            len: metadata.len(),
        })
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct WriteFileArgs {
    #[serde(alias = "file", alias = "filepath")]
    pub(super) path: String,
    pub(super) content: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct EditFileArgs {
    #[serde(alias = "file", alias = "filepath")]
    pub(super) path: String,
    #[serde(
        alias = "old",
        alias = "old_text",
        alias = "oldText",
        alias = "search",
        alias = "from"
    )]
    pub(super) old_string: String,
    #[serde(
        alias = "new",
        alias = "new_text",
        alias = "newText",
        alias = "replace",
        alias = "to"
    )]
    pub(super) new_string: String,
    #[serde(alias = "all")]
    pub(super) replace_all: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct LockPathsArgs {
    pub(super) globs: Vec<String>,
    pub(super) ttl_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct UnlockPathsArgs {
    pub(super) tokens: Vec<String>,
}

impl Tools {
    pub(super) fn record_seen_file(&self, path: &Path, stamp: std::io::Result<FileStamp>) {
        if let Ok(stamp) = stamp {
            let key = crate::util::resolve_path(path);
            self.seen_files
                .lock()
                .expect("seen files lock")
                .entry(self.session_id.clone())
                .or_default()
                .insert(key, stamp);
        }
    }

    fn check_seen_file(
        &self,
        path: &Path,
        display: &str,
        current: &std::io::Result<FileStamp>,
    ) -> Result<()> {
        let key = crate::util::resolve_path(path);
        let seen = self.seen_files.lock().expect("seen files lock");
        let previous = seen.get(&self.session_id).and_then(|files| files.get(&key));
        if previous.is_some_and(|stamp| current.as_ref().ok() != Some(stamp)) {
            anyhow::bail!(
                "{display} changed since it was last read here. Read it again before writing."
            );
        }
        Ok(())
    }

    pub(super) async fn enforce_write_access(&self, rel: &str) -> Result<()> {
        if let (Some(manager), Some(agent_id)) = (&self.manager, &self.agent_id) {
            // 1. Check path access
            if !manager.is_path_allowed(&self.cwd(), agent_id, rel).await {
                anyhow::bail!(
                    "Path {} is outside the allowed WorkScope for agent {}",
                    rel,
                    agent_id
                );
            }

            // 2. Check locks
            let locked_by_other = manager.locks.lock().await.is_locked_by_other(agent_id, rel);
            if locked_by_other {
                anyhow::bail!("Path {} is locked by another agent", rel);
            }

            // Live working-place map for active-path UI (in-memory source of truth).
            if self.run_id.is_some() {
                manager
                    .upsert_working_place(
                        &self.root.to_string_lossy(),
                        agent_id,
                        rel,
                        self.run_id.clone(),
                    )
                    .await;
            }
        }
        Ok(())
    }

    pub(super) async fn write_file(&self, args: WriteFileArgs) -> Result<ToolResult> {
        let expanded = expand_tilde(&args.path);
        let abs_path = Path::new(&expanded);

        // Absolute paths are written directly (permission was already approved upstream).
        // Relative paths are resolved against the workspace root.
        let (target, display) = if abs_path.is_absolute() {
            (abs_path.to_path_buf(), args.path.clone())
        } else {
            let rel = sanitize_rel_path(&self.cwd(), &args.path)?;
            self.enforce_write_access(&rel).await?;
            let p = self.cwd().join(&rel);
            (p, rel)
        };

        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        if target.exists() {
            let stamp = FileStamp::at(&target);
            let identical = tokio::fs::read_to_string(&target)
                .await
                .is_ok_and(|existing| existing == args.content);
            let current = FileStamp::at(&target);
            self.check_seen_file(&target, &display, &current)?;
            if identical {
                self.record_seen_file(&target, stamp);
                return Ok(ToolResult::Success(format!(
                    "File unchanged (content identical): {}",
                    display
                )));
            }
        }

        let bytes = args.content.len();
        tokio::fs::write(&target, &args.content).await?;
        self.record_seen_file(&target, FileStamp::at(&target));
        Ok(ToolResult::Success(format!(
            "File written: {} ({} bytes)",
            display, bytes
        )))
    }

    pub(super) async fn edit_file(&self, args: EditFileArgs) -> Result<ToolResult> {
        if args.old_string.is_empty() {
            anyhow::bail!("old_string must not be empty");
        }

        let expanded = expand_tilde(&args.path);
        let abs_path = Path::new(&expanded);

        // Absolute paths are used directly (permission was already approved upstream).
        // Relative paths are resolved against the workspace root.
        let (target, display) = if abs_path.is_absolute() {
            (abs_path.to_path_buf(), args.path.clone())
        } else {
            let rel = sanitize_rel_path(&self.cwd(), &args.path)?;
            self.enforce_write_access(&rel).await?;
            let p = self.cwd().join(&rel);
            (p, rel)
        };

        if !target.exists() {
            anyhow::bail!("file not found: {}", display);
        }
        if target.is_dir() {
            anyhow::bail!(
                "path '{}' is a directory. Use Glob to enumerate files, then Edit with an exact file path.",
                display
            );
        }

        let stamp = FileStamp::at(&target);
        let existing = tokio::fs::read_to_string(&target).await?;
        let match_count = existing.matches(&args.old_string).count();
        if match_count == 0 {
            anyhow::bail!("old_string was not found in file: {}", display);
        }

        let replace_all = args.replace_all.unwrap_or(false);
        if match_count > 1 && !replace_all {
            anyhow::bail!(
                "old_string matched {} locations in {}. Provide a more specific old_string or set replace_all=true.",
                match_count,
                display
            );
        }

        let updated = if replace_all {
            existing.replace(&args.old_string, &args.new_string)
        } else {
            existing.replacen(&args.old_string, &args.new_string, 1)
        };

        if updated == existing {
            self.record_seen_file(&target, stamp);
            return Ok(ToolResult::Success(format!(
                "File unchanged (no effective edit): {}",
                display
            )));
        }

        tokio::fs::write(&target, updated).await?;
        self.record_seen_file(&target, FileStamp::at(&target));
        let replaced = if replace_all { match_count } else { 1 };
        Ok(ToolResult::Success(format!(
            "Edited file: {} ({} replacement{})",
            display,
            replaced,
            if replaced == 1 { "" } else { "s" }
        )))
    }

    pub(super) async fn lock_paths(&self, args: LockPathsArgs) -> Result<ToolResult> {
        let (manager, agent_id) = match (&self.manager, &self.agent_id) {
            (Some(m), Some(id)) => (m, id),
            _ => anyhow::bail!("Locking requires AgentManager context"),
        };

        let ttl = Duration::from_millis(args.ttl_ms.unwrap_or(300_000)); // Default 5 min
        let res = manager
            .locks
            .lock()
            .await
            .acquire(agent_id, args.globs, ttl);

        Ok(ToolResult::LockResult {
            acquired: res.acquired,
            denied: res.denied,
        })
    }

    pub(super) async fn unlock_paths(&self, args: UnlockPathsArgs) -> Result<ToolResult> {
        let (manager, agent_id) = match (&self.manager, &self.agent_id) {
            (Some(m), Some(id)) => (m, id),
            _ => anyhow::bail!("Locking requires AgentManager context"),
        };

        manager.locks.lock().await.release(agent_id, args.tokens);
        Ok(ToolResult::Success("Locks released".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::super::file_tools::ReadFileArgs;
    use super::*;

    fn write(path: &str, content: &str) -> WriteFileArgs {
        WriteFileArgs {
            path: path.into(),
            content: content.into(),
        }
    }

    async fn read(tools: &Tools, path: &str) {
        tools
            .read_file(ReadFileArgs {
                path: path.into(),
                max_bytes: None,
                line_range: None,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn write_refuses_an_external_change_until_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.txt");
        let tools = Tools::new(dir.path().to_path_buf()).unwrap();
        std::fs::write(&path, "before").unwrap();
        read(&tools, path.to_str().unwrap()).await;
        std::fs::write(&path, "another writer's change").unwrap();

        let err = tools
            .write_file(write("note.txt", "mine"))
            .await
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "note.txt changed since it was last read here. Read it again before writing."
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "another writer's change"
        );

        read(&tools, "note.txt").await;
        tools.write_file(write("note.txt", "mine")).await.unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine");
    }

    #[tokio::test]
    async fn write_refuses_changes_since_its_own_write_or_edit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.txt");
        let tools = Tools::new(dir.path().to_path_buf()).unwrap();
        tools.write_file(write("note.txt", "new")).await.unwrap();
        tools
            .write_file(write("note.txt", "own write"))
            .await
            .unwrap();
        std::fs::write(&path, "external change after Write").unwrap();
        assert!(tools.write_file(write("note.txt", "stale")).await.is_err());

        tools
            .edit_file(EditFileArgs {
                path: "note.txt".into(),
                old_string: "after Write".into(),
                new_string: "accepted by Edit".into(),
                replace_all: None,
            })
            .await
            .unwrap();
        tools
            .write_file(write("note.txt", "after edit"))
            .await
            .unwrap();
        tools
            .edit_file(EditFileArgs {
                path: "note.txt".into(),
                old_string: "after edit".into(),
                new_string: "new edit".into(),
                replace_all: None,
            })
            .await
            .unwrap();
        std::fs::write(&path, "external change after Edit").unwrap();
        assert!(tools.write_file(write("note.txt", "stale")).await.is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "external change after Edit"
        );
    }

    #[tokio::test]
    async fn an_unseen_file_is_writable_in_a_separate_engine() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.txt");
        let parent = Tools::new(dir.path().to_path_buf()).unwrap();
        let child = Tools::new(dir.path().to_path_buf()).unwrap();
        std::fs::write(&path, "before").unwrap();
        read(&parent, "note.txt").await;
        child
            .write_file(write("note.txt", "child changed it"))
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "child changed it");
        assert!(parent.write_file(write("note.txt", "stale")).await.is_err());
    }
}

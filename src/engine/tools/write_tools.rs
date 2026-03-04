use super::block_on_async;
use super::tool_helpers::{expand_tilde, sanitize_rel_path};
use super::{ToolResult, Tools};
use anyhow::Result;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

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
    pub(super) fn enforce_write_access(&self, rel: &str) -> Result<()> {
        if let (Some(manager), Some(agent_id)) = (&self.manager, &self.agent_id) {
            // 1. Check work_globs
            let allowed = block_on_async(async {
                manager.is_path_allowed(&self.root, agent_id, rel).await
            });

            if !allowed {
                anyhow::bail!(
                    "Path {} is outside the allowed WorkScope for agent {}",
                    rel, agent_id
                );
            }

            // 2. Check locks
            let locked_by_other = block_on_async(async {
                manager.locks.lock().await.is_locked_by_other(agent_id, &rel)
            });

            if locked_by_other {
                anyhow::bail!("Path {} is locked by another agent", rel);
            }

            // Live working-place map for active-path UI (in-memory source of truth).
            if self.run_id.is_some() {
                let repo_path = self.root.to_string_lossy().to_string();
                let run_id = self.run_id.clone();
                let rel_for_map = rel.to_string();
                let agent_for_map = agent_id.clone();
                block_on_async(async {
                    manager
                        .upsert_working_place(&repo_path, &agent_for_map, &rel_for_map, run_id)
                        .await;
                });
            }
        }
        Ok(())
    }

    pub(super) fn write_file(&self, args: WriteFileArgs) -> Result<ToolResult> {
        let expanded = expand_tilde(&args.path);
        let abs_path = Path::new(&expanded);

        // Absolute paths are written directly (permission was already approved upstream).
        // Relative paths are resolved against the workspace root.
        let (target, display) = if abs_path.is_absolute() {
            (abs_path.to_path_buf(), args.path.clone())
        } else {
            let rel = sanitize_rel_path(&self.root, &args.path)?;
            self.enforce_write_access(&rel)?;
            let p = self.root.join(&rel);
            (p, rel)
        };

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        if target.exists() {
            match fs::read_to_string(&target) {
                Ok(existing) if existing == args.content => {
                    return Ok(ToolResult::Success(format!(
                        "File unchanged (content identical): {}",
                        display
                    )));
                }
                Ok(_) => {} // content differs, proceed to write
                Err(_) => {} // file unreadable (e.g. binary), proceed to overwrite
            }
        }

        let bytes = args.content.len();
        fs::write(&target, &args.content)?;
        Ok(ToolResult::Success(format!(
            "File written: {} ({} bytes)",
            display, bytes
        )))
    }

    pub(super) fn edit_file(&self, args: EditFileArgs) -> Result<ToolResult> {
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
            let rel = sanitize_rel_path(&self.root, &args.path)?;
            self.enforce_write_access(&rel)?;
            let p = self.root.join(&rel);
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

        let existing = fs::read_to_string(&target)?;
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
            return Ok(ToolResult::Success(format!(
                "File unchanged (no effective edit): {}",
                display
            )));
        }

        fs::write(&target, updated)?;
        let replaced = if replace_all { match_count } else { 1 };
        Ok(ToolResult::Success(format!(
            "Edited file: {} ({} replacement{})",
            display,
            replaced,
            if replaced == 1 { "" } else { "s" }
        )))
    }

    pub(super) fn lock_paths(&self, args: LockPathsArgs) -> Result<ToolResult> {
        let (manager, agent_id) = match (&self.manager, &self.agent_id) {
            (Some(m), Some(id)) => (m, id),
            _ => anyhow::bail!("Locking requires AgentManager context"),
        };

        let ttl = Duration::from_millis(args.ttl_ms.unwrap_or(300000)); // Default 5 min
        let res = block_on_async(async {
            manager.locks.lock().await.acquire(agent_id, args.globs, ttl)
        });

        Ok(ToolResult::LockResult {
            acquired: res.acquired,
            denied: res.denied,
        })
    }

    pub(super) fn unlock_paths(&self, args: UnlockPathsArgs) -> Result<ToolResult> {
        let (manager, agent_id) = match (&self.manager, &self.agent_id) {
            (Some(m), Some(id)) => (m, id),
            _ => anyhow::bail!("Locking requires AgentManager context"),
        };

        block_on_async(async {
            manager.locks.lock().await.release(agent_id, args.tokens)
        });

        Ok(ToolResult::Success("Locks released".to_string()))
    }
}

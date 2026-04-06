pub mod sessions;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub use sessions::SessionStore;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StateFile {
    #[serde(rename = "pm_task")]
    PmTask {
        id: String,
        status: String,
        #[serde(default)]
        assigned_tasks: Vec<String>,
    },
    #[serde(rename = "user_stories")]
    UserStories { id: String },
    #[serde(rename = "coder_task")]
    CoderTask {
        id: String,
        status: String,
        story_id: Option<String>,
        assigned_to: String,
    },
    #[serde(rename = "message")]
    Message {
        id: String,
        from: String,
        to: String,
        ts: u64,
        #[serde(default)]
        task_id: Option<String>,
    },
}

pub struct StateFs {
    root: PathBuf,
}

impl StateFs {
    pub fn new(ws_root: PathBuf) -> Self {
        let root = ws_root.join(".linggen/workspace");
        // Compat: warn if old directory exists but new one doesn't
        let old_root = ws_root.join(".linggen-agent/workspace");
        if old_root.exists() && !root.exists() {
            tracing::warn!(
                "Legacy state directory exists at {}; new location is {}. Please migrate manually.",
                old_root.display(),
                root.display()
            );
        }
        Self { root }
    }

    /// Resolve a relative path and verify it stays within the state root.
    fn safe_resolve(&self, rel_path: &str) -> Result<PathBuf> {
        let path = self.root.join(rel_path);
        let canonical = path
            .canonicalize()
            .or_else(|_| {
                // File might not exist yet (write case). Canonicalize the parent instead.
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                    let canon_parent = parent.canonicalize()?;
                    Ok(canon_parent.join(path.file_name().unwrap_or_default()))
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "cannot resolve path",
                    ))
                }
            })?;
        let canon_root = self.root.canonicalize().unwrap_or_else(|_| self.root.clone());
        if !canonical.starts_with(&canon_root) {
            anyhow::bail!(
                "Path traversal rejected: {} escapes state root {}",
                rel_path,
                canon_root.display()
            );
        }
        Ok(canonical)
    }

    pub fn read_file(&self, rel_path: &str) -> Result<(StateFile, String)> {
        let path = self.root.join(rel_path);
        if !path.exists() {
            anyhow::bail!("State file not found: {}", rel_path);
        }
        let content = fs::read_to_string(&path)?;
        self.parse_markdown(&content)
    }

    pub fn write_file(&self, rel_path: &str, state: &StateFile, body: &str) -> Result<()> {
        let path = self.safe_resolve(rel_path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let yaml = serde_yml::to_string(state)?;
        let content = format!("---\n{}---\n\n{}", yaml, body);
        fs::write(path, content)?;
        Ok(())
    }

    pub fn list_tasks(&self) -> Result<Vec<(StateFile, String)>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let tasks_dir = self.root.join("tasks");
        if !tasks_dir.exists() {
            return Ok(Vec::new());
        }

        let mut tasks = Vec::new();
        for entry in fs::read_dir(tasks_dir)? {
            let entry = entry?;
            if entry.path().extension().map_or(false, |ext| ext == "md") {
                let content = fs::read_to_string(entry.path())?;
                if let Ok(parsed) = self.parse_markdown(&content) {
                    tasks.push(parsed);
                }
            }
        }
        Ok(tasks)
    }

    fn parse_markdown(&self, content: &str) -> Result<(StateFile, String)> {
        if !content.starts_with("---") {
            anyhow::bail!("Markdown must start with YAML frontmatter (---)");
        }
        let parts: Vec<&str> = content.splitn(3, "---").collect();
        if parts.len() < 3 {
            anyhow::bail!("Markdown missing closing frontmatter delimiter (---)");
        }
        let state: StateFile = serde_yml::from_str(parts[1])?;
        let body = parts[2].trim().to_string();
        Ok((state, body))
    }
}

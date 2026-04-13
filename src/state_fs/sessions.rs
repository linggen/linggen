use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

/// Flat-file session and chat message store.
///
/// Directory layout:
/// ```text
/// <project>/.linggen/sessions/
///   <session_id>/
///     session.yaml      # SessionMeta
///     messages.jsonl     # one ChatMsg per line, append-only
/// ```
pub struct SessionStore {
    sessions_dir: PathBuf,
}

fn default_creator() -> String {
    "user".to_string()
}

fn is_default_creator(s: &str) -> bool {
    s == "user"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    pub created_at: u64,
    /// When set, this skill is bound to the session and activated on every message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
    /// Who created this session: "user", "skill", "mission", "agent"
    #[serde(default = "default_creator", skip_serializing_if = "is_default_creator")]
    pub creator: String,
    /// Session-level model override. Persisted so it survives reload/session switch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    /// Current working directory of the agent in this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Detected git root path when agent is inside a project. None in home mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Display name — last segment of git root (e.g. "linggen"). None in home mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    /// Originating mission ID (when creator is "mission").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_id: Option<String>,
    /// User ID of the session creator (owner or consumer's linggen.dev user_id).
    /// Used to isolate sessions by user in proxy rooms.
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "consumer_user_id")]
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMsg {
    pub agent_id: String,
    pub from_id: String,
    pub to_id: String,
    pub content: String,
    pub timestamp: u64,
    pub is_observation: bool,
}

impl SessionStore {
    /// Create a store with an explicit sessions directory (for ProjectStore).
    pub fn with_sessions_dir(sessions_dir: PathBuf) -> Self {
        Self { sessions_dir }
    }

    // ------------------------------------------------------------------
    // Session CRUD
    // ------------------------------------------------------------------

    pub fn add_session(&self, meta: &SessionMeta) -> Result<()> {
        Self::validate_id(&meta.id)?;
        let dir = self.session_dir(&meta.id);
        fs::create_dir_all(&dir)?;
        let yaml = serde_yml::to_string(meta)?;
        fs::write(dir.join("session.yaml"), yaml)?;
        // Create empty messages file
        let msgs_path = dir.join("messages.jsonl");
        if !msgs_path.exists() {
            fs::write(&msgs_path, "")?;
        }
        Ok(())
    }

    pub fn get_session_meta(&self, session_id: &str) -> Result<Option<SessionMeta>> {
        let yaml_path = self.session_dir(session_id).join("session.yaml");
        if !yaml_path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&yaml_path)?;
        let meta: SessionMeta = serde_yml::from_str(&content)?;
        Ok(Some(meta))
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionMeta>> {
        self.list_sessions_paginated(None, None)
    }

    /// List sessions with optional pagination. Results are sorted newest-first.
    /// `limit` caps how many are returned; `offset` skips that many from the top.
    pub fn list_sessions_paginated(
        &self,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<SessionMeta>> {
        if !self.sessions_dir.exists() {
            return Ok(Vec::new());
        }
        let mut sessions = Vec::new();
        for entry in fs::read_dir(&self.sessions_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let yaml_path = entry.path().join("session.yaml");
            if yaml_path.exists() {
                let content = fs::read_to_string(&yaml_path)?;
                match serde_yml::from_str::<SessionMeta>(&content) {
                    Ok(meta) => sessions.push(meta),
                    Err(e) => {
                        tracing::warn!(
                            "Skipping corrupt session.yaml at {}: {}",
                            yaml_path.display(),
                            e
                        );
                    }
                }
            }
        }
        sessions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        let off = offset.unwrap_or(0);
        if off > 0 {
            sessions = sessions.into_iter().skip(off).collect();
        }
        if let Some(lim) = limit {
            sessions.truncate(lim);
        }
        Ok(sessions)
    }

    /// Return total session count without reading YAML files (just counts directories).
    pub fn count_sessions(&self) -> usize {
        if !self.sessions_dir.exists() {
            return 0;
        }
        fs::read_dir(&self.sessions_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
                    .filter(|e| e.path().join("session.yaml").exists())
                    .count()
            })
            .unwrap_or(0)
    }

    /// Check whether a session has any chat messages (without loading them all).
    pub fn session_has_messages(&self, session_id: &str) -> bool {
        let msgs_path = self.session_dir(session_id).join("messages.jsonl");
        if !msgs_path.exists() {
            return false;
        }
        // Check if file has any non-empty lines
        if let Ok(file) = fs::File::open(&msgs_path) {
            let reader = BufReader::new(file);
            for line in reader.lines() {
                if let Ok(l) = line {
                    if !l.trim().is_empty() {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        Self::validate_id(session_id)?;
        let yaml_path = self.session_dir(session_id).join("session.yaml");
        if !yaml_path.exists() {
            bail!("Session not found: {}", session_id);
        }
        let content = fs::read_to_string(&yaml_path)?;
        let mut meta: SessionMeta = serde_yml::from_str(&content)?;
        meta.title = new_title.to_string();
        let yaml = serde_yml::to_string(&meta)?;
        fs::write(yaml_path, yaml)?;
        Ok(())
    }

    pub fn update_session_meta(&self, meta: &SessionMeta) -> Result<()> {
        Self::validate_id(&meta.id)?;
        let yaml_path = self.session_dir(&meta.id).join("session.yaml");
        if !yaml_path.exists() {
            bail!("Session not found: {}", meta.id);
        }
        let yaml = serde_yml::to_string(meta)?;
        fs::write(yaml_path, yaml)?;
        Ok(())
    }

    pub fn remove_session(&self, session_id: &str) -> Result<()> {
        Self::validate_id(session_id)?;
        let dir = self.session_dir(session_id);
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Chat messages
    // ------------------------------------------------------------------

    pub fn add_chat_message(&self, session_id: &str, msg: &ChatMsg) -> Result<()> {
        Self::validate_id(session_id)?;
        let dir = self.session_dir(session_id);
        fs::create_dir_all(&dir)?;
        let msgs_path = dir.join("messages.jsonl");
        let line = serde_json::to_string(msg)?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(msgs_path)?;
        writeln!(file, "{}", line)?;
        Ok(())
    }

    pub fn get_chat_history(
        &self,
        session_id: &str,
    ) -> Result<Vec<ChatMsg>> {
        Self::validate_id(session_id)?;
        let msgs_path = self.session_dir(session_id).join("messages.jsonl");
        if !msgs_path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(msgs_path)?;
        let reader = BufReader::new(file);
        let mut messages = Vec::new();
        for line in reader.lines() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<ChatMsg>(trimmed) {
                Ok(msg) => {
                    messages.push(msg);
                }
                Err(e) => {
                    tracing::warn!("Skipping corrupt JSONL line: {}", e);
                }
            }
        }
        Ok(messages)
    }

    /// Replace the last plan message in the session's messages.jsonl.
    /// A plan message has content containing `"type":"plan"`.
    pub fn update_last_plan_message(&self, session_id: &str, updated: &ChatMsg) -> Result<bool> {
        Self::validate_id(session_id)?;
        let msgs_path = self.session_dir(session_id).join("messages.jsonl");
        if !msgs_path.exists() {
            return Ok(false);
        }
        let file = fs::File::open(&msgs_path)?;
        let reader = BufReader::new(file);
        let mut lines: Vec<String> = Vec::new();
        let mut last_plan_idx: Option<usize> = None;
        for line in reader.lines() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() { continue; }
            if trimmed.contains("\"type\":\"plan\"") && trimmed.contains("\"plan\":{") {
                last_plan_idx = Some(lines.len());
            }
            lines.push(line);
        }
        let Some(idx) = last_plan_idx else { return Ok(false) };
        lines[idx] = serde_json::to_string(updated)?;
        let tmp_path = msgs_path.with_extension("jsonl.tmp");
        {
            let mut tmp = fs::File::create(&tmp_path)?;
            for l in &lines {
                writeln!(tmp, "{}", l)?;
            }
        }
        fs::rename(&tmp_path, &msgs_path)?;
        Ok(true)
    }

    pub fn clear_chat_history(&self, session_id: &str) -> Result<usize> {
        Self::validate_id(session_id)?;
        let msgs_path = self.session_dir(session_id).join("messages.jsonl");
        if !msgs_path.exists() {
            return Ok(0);
        }
        // Count existing lines first
        let file = fs::File::open(&msgs_path)?;
        let count = BufReader::new(file)
            .lines()
            .filter_map(|l| l.ok())
            .filter(|l| !l.trim().is_empty())
            .count();
        // Truncate
        fs::write(&msgs_path, "")?;
        Ok(count)
    }

    /// Replace the entire chat history for a session with the given messages.
    pub fn rewrite_chat_history(&self, session_id: &str, msgs: &[ChatMsg]) -> Result<()> {
        Self::validate_id(session_id)?;
        let dir = self.session_dir(session_id);
        fs::create_dir_all(&dir)?;
        let msgs_path = dir.join("messages.jsonl");
        let mut buf = String::new();
        for msg in msgs {
            let line = serde_json::to_string(msg)?;
            buf.push_str(&line);
            buf.push('\n');
        }
        fs::write(msgs_path, buf)?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    pub fn session_dir(&self, session_id: &str) -> PathBuf {
        self.sessions_dir.join(session_id)
    }

    fn validate_id(id: &str) -> Result<()> {
        if id.is_empty() {
            bail!("Session ID must not be empty");
        }
        if id.contains("..") || id.contains('/') || id.contains('\\') {
            bail!("Session ID contains invalid characters: {}", id);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (SessionStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::with_sessions_dir(dir.path().join(".linggen/sessions"));
        (store, dir)
    }

    #[test]
    fn test_session_crud() {
        let (store, _dir) = temp_store();
        let meta = SessionMeta {
            id: "sess-1000-abcd1234".into(),
            title: "Test Session".into(),
            created_at: 1000,
            skill: None,
            creator: "user".into(),
            cwd: None, project: None, project_name: None, mission_id: None, model_id: None, user_id: None,
        };
        store.add_session(&meta).unwrap();

        let sessions = store.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "Test Session");
        assert_eq!(sessions[0].id, "sess-1000-abcd1234");

        store
            .rename_session("sess-1000-abcd1234", "Renamed")
            .unwrap();
        let sessions = store.list_sessions().unwrap();
        assert_eq!(sessions[0].title, "Renamed");

        store.remove_session("sess-1000-abcd1234").unwrap();
        assert_eq!(store.list_sessions().unwrap().len(), 0);
    }

    #[test]
    fn test_list_sessions_sorted_desc() {
        let (store, _dir) = temp_store();
        for (id, ts) in [("s1", 100u64), ("s2", 300), ("s3", 200)] {
            store
                .add_session(&SessionMeta {
                    id: id.into(),
                    title: id.into(),
                    created_at: ts,
                    skill: None,
                    creator: "user".into(),
                    cwd: None, project: None, project_name: None, mission_id: None, model_id: None, user_id: None,
                })
                .unwrap();
        }
        let sessions = store.list_sessions().unwrap();
        let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["s2", "s3", "s1"]);
    }

    #[test]
    fn test_chat_messages_roundtrip() {
        let (store, _dir) = temp_store();
        let meta = SessionMeta {
            id: "s1".into(),
            title: "t".into(),
            created_at: 1000,
            skill: None,
            creator: "user".into(),
            cwd: None, project: None, project_name: None, mission_id: None, model_id: None, user_id: None,
        };
        store.add_session(&meta).unwrap();

        let msg1 = ChatMsg {
            agent_id: "ling".into(),
            from_id: "user".into(),
            to_id: "ling".into(),
            content: "Hello".into(),
            timestamp: 1000,
            is_observation: false,
        };
        let msg2 = ChatMsg {
            agent_id: "ling".into(),
            from_id: "ling".into(),
            to_id: "user".into(),
            content: "Hi there".into(),
            timestamp: 1001,
            is_observation: false,
        };
        store.add_chat_message("s1", &msg1).unwrap();
        store.add_chat_message("s1", &msg2).unwrap();

        let history = store.get_chat_history("s1").unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content, "Hello");
        assert_eq!(history[1].content, "Hi there");
    }

    #[test]
    fn test_chat_loads_all_agents_in_session() {
        let (store, _dir) = temp_store();
        store
            .add_session(&SessionMeta {
                id: "s1".into(),
                title: "t".into(),
                created_at: 1000,
                skill: None,
                creator: "user".into(),
                cwd: None, project: None, project_name: None, mission_id: None, model_id: None, user_id: None,
            })
            .unwrap();

        store
            .add_chat_message(
                "s1",
                &ChatMsg {
                    agent_id: "ling".into(),
                    from_id: "user".into(),
                    to_id: "ling".into(),
                    content: "for ling".into(),
                    timestamp: 1000,
                    is_observation: false,
                },
            )
            .unwrap();
        store
            .add_chat_message(
                "s1",
                &ChatMsg {
                    agent_id: "coder".into(),
                    from_id: "user".into(),
                    to_id: "coder".into(),
                    content: "for coder".into(),
                    timestamp: 1001,
                    is_observation: false,
                },
            )
            .unwrap();

        // Session loads all messages regardless of agent_id
        let all_msgs = store.get_chat_history("s1").unwrap();
        assert_eq!(all_msgs.len(), 2);
        assert_eq!(all_msgs[0].content, "for ling");
        assert_eq!(all_msgs[1].content, "for coder");
    }

    #[test]
    fn test_clear_chat_history() {
        let (store, _dir) = temp_store();
        store
            .add_session(&SessionMeta {
                id: "s1".into(),
                title: "t".into(),
                created_at: 1000,
                skill: None,
                creator: "user".into(),
                cwd: None, project: None, project_name: None, mission_id: None, model_id: None, user_id: None,
            })
            .unwrap();
        store
            .add_chat_message(
                "s1",
                &ChatMsg {
                    agent_id: "ling".into(),
                    from_id: "user".into(),
                    to_id: "ling".into(),
                    content: "hello".into(),
                    timestamp: 1000,
                    is_observation: false,
                },
            )
            .unwrap();

        let cleared = store.clear_chat_history("s1").unwrap();
        assert_eq!(cleared, 1);
        assert_eq!(store.get_chat_history("s1").unwrap().len(), 0);
    }

    #[test]
    fn test_remove_session_deletes_messages() {
        let (store, _dir) = temp_store();
        store
            .add_session(&SessionMeta {
                id: "s1".into(),
                title: "t".into(),
                created_at: 1000,
                skill: None,
                creator: "user".into(),
                cwd: None, project: None, project_name: None, mission_id: None, model_id: None, user_id: None,
            })
            .unwrap();
        store
            .add_chat_message(
                "s1",
                &ChatMsg {
                    agent_id: "ling".into(),
                    from_id: "user".into(),
                    to_id: "ling".into(),
                    content: "hello".into(),
                    timestamp: 1000,
                    is_observation: false,
                },
            )
            .unwrap();

        store.remove_session("s1").unwrap();
        // Messages file is gone with the directory
        assert!(store.get_chat_history("s1").unwrap().is_empty());
    }

    #[test]
    fn test_invalid_session_id_rejected() {
        let (store, _dir) = temp_store();
        assert!(store
            .add_session(&SessionMeta {
                id: "../escape".into(),
                title: "t".into(),
                created_at: 1000,
                skill: None,
                creator: "user".into(),
                cwd: None, project: None, project_name: None, mission_id: None, model_id: None, user_id: None,
            })
            .is_err());
        assert!(store
            .add_session(&SessionMeta {
                id: "a/b".into(),
                title: "t".into(),
                created_at: 1000,
                skill: None,
                creator: "user".into(),
                cwd: None, project: None, project_name: None, mission_id: None, model_id: None, user_id: None,
            })
            .is_err());
        assert!(store
            .add_session(&SessionMeta {
                id: "".into(),
                title: "t".into(),
                created_at: 1000,
                skill: None,
                creator: "user".into(),
                cwd: None, project: None, project_name: None, mission_id: None, model_id: None, user_id: None,
            })
            .is_err());
    }

    #[test]
    fn test_add_message_creates_session_dir() {
        let (store, _dir) = temp_store();
        // add_chat_message should work even if add_session wasn't called
        store
            .add_chat_message(
                "auto-created",
                &ChatMsg {
                    agent_id: "ling".into(),
                    from_id: "user".into(),
                    to_id: "ling".into(),
                    content: "hello".into(),
                    timestamp: 1000,
                    is_observation: false,
                },
            )
            .unwrap();
        let history = store.get_chat_history("auto-created").unwrap();
        assert_eq!(history.len(), 1);
    }
}

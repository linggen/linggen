//! The session ledger: what each model used in a session, summed over its
//! calls, kept as `usage.json` in the session's folder. The one place usage
//! is kept per session and model; the chat's top bar and `agent_status`
//! read it through `ModelManager::spend_of`, which adds a price when one is
//! known. Written once per model call, from `AgentEngine::note_call` (and
//! the next-prompt fork) only.

use super::SessionStore;
use crate::provider::models::{ModelTally, TokenUsage};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Mutex;

const FILE: &str = "usage.json";

/// Subagents and forks write one session's ledger at the same time: a
/// read-modify-write is held under this lock.
static WRITING: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionUsage {
    /// Model id → what it used here.
    pub models: BTreeMap<String, ModelTally>,
}

impl SessionUsage {
    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }
}

impl SessionStore {
    /// Add one model call to the session's ledger. A session with no folder
    /// (a call made outside any session) records nothing.
    pub fn record_usage(&self, session_id: &str, model_id: &str, usage: Option<&TokenUsage>) {
        if let Err(e) = self.try_record_usage(session_id, model_id, usage) {
            tracing::warn!("usage of '{model_id}' not recorded for session {session_id}: {e:#}");
        }
    }

    fn try_record_usage(
        &self,
        session_id: &str,
        model_id: &str,
        usage: Option<&TokenUsage>,
    ) -> anyhow::Result<()> {
        if session_id.is_empty() || session_id.contains(['/', '\\']) || session_id.contains("..") {
            return Ok(());
        }
        let dir = self.session_dir(session_id);
        if !dir.is_dir() {
            return Ok(());
        }
        let _held = WRITING.lock().unwrap_or_else(|e| e.into_inner());
        let mut ledger = self.session_usage(session_id);
        ledger
            .models
            .entry(model_id.to_string())
            .or_default()
            .add(usage);
        let mut temp = tempfile::NamedTempFile::new_in(&dir)?;
        std::io::Write::write_all(&mut temp, &serde_json::to_vec(&ledger)?)?;
        temp.persist(dir.join(FILE))?;
        Ok(())
    }

    /// What the session's models used so far; empty when none ran.
    pub fn session_usage(&self, session_id: &str) -> SessionUsage {
        if session_id.is_empty() || session_id.contains(['/', '\\']) || session_id.contains("..") {
            return SessionUsage::default();
        }
        std::fs::read(self.session_dir(session_id).join(FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, SessionStore) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("s1")).unwrap();
        let store = SessionStore::with_sessions_dir(dir.path().to_path_buf());
        (dir, store)
    }

    fn call(prompt: usize, read: usize, write: usize, out: usize) -> TokenUsage {
        TokenUsage {
            prompt_tokens: Some(prompt),
            completion_tokens: Some(out),
            total_tokens: None,
            cached_tokens: Some(read),
            cache_write_tokens: Some(write),
        }
    }

    #[test]
    fn calls_add_up_per_session_and_model() {
        let (_dir, store) = store();
        store.record_usage("s1", "sonnet", Some(&call(1_000, 600, 100, 50)));
        store.record_usage("s1", "sonnet", Some(&call(2_000, 0, 0, 70)));
        store.record_usage("s1", "gemini", Some(&call(10, 0, 0, 5)));
        store.record_usage("s1", "gemini", None);
        let ledger = store.session_usage("s1");
        let sonnet = &ledger.models["sonnet"];
        assert_eq!(sonnet.calls, 2);
        assert_eq!(sonnet.tokens.input, 300 + 2_000);
        assert_eq!(sonnet.tokens.cache_read, 600);
        assert_eq!(sonnet.tokens.cache_write, 100);
        assert_eq!(sonnet.tokens.output, 120);
        let gemini = &ledger.models["gemini"];
        assert_eq!((gemini.calls, gemini.unreported), (2, 1));
        assert!(store.session_usage("other").is_empty());
    }

    #[test]
    fn a_call_outside_any_session_folder_records_nothing() {
        let (dir, store) = store();
        store.record_usage("no-such", "m", Some(&call(1, 0, 0, 1)));
        store.record_usage("../escape", "m", Some(&call(1, 0, 0, 1)));
        assert!(!dir.path().join("no-such").exists());
        assert!(!dir.path().parent().unwrap().join("escape").exists());
    }

    #[test]
    fn calls_made_at_once_are_all_kept() {
        let (_dir, store) = store();
        let store = std::sync::Arc::new(store);
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let store = store.clone();
                std::thread::spawn(move || {
                    for _ in 0..10 {
                        store.record_usage("s1", "m", Some(&call(10, 0, 0, 1)));
                    }
                })
            })
            .collect();
        threads.into_iter().for_each(|t| t.join().unwrap());
        assert_eq!(store.session_usage("s1").models["m"].calls, 80);
    }
}

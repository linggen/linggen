//! The session ledger: what each model used in a session, summed over its
//! calls, kept as `usage.json` in the session's folder. The one place usage
//! is kept per session and model; the chat's top bar and `agent_status`
//! read it through `ModelManager::spend_of`, which adds a price when one is
//! known. Written once per model call, through `engine::spend` only: its
//! `Seal` is the key `record_usage` demands.

use super::devices::write_atomic;
use super::SessionStore;
use crate::provider::models::{Billing, ModelTally, TokenUsage};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const FILE: &str = "usage.json";

/// What `record_usage` demands to be called with: a key. Its one implementor
/// is `engine::spend::Seal`, whose field is private to that module — so the
/// ledger is written from there only, and a second writer does not compile
/// unless it makes a key of its own, which `engine::spend`'s tripwire test
/// counts. (A trait, not the type itself: the store does not name the
/// engine above it.)
pub trait LedgerKey {}

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

fn valid(session_id: &str) -> bool {
    !session_id.is_empty() && !session_id.contains(['/', '\\']) && !session_id.contains("..")
}

impl SessionStore {
    /// Add one model call to the session's ledger. A session with no folder
    /// (a call made outside any session, or one removed meanwhile) records
    /// nothing.
    pub fn record_usage<K: LedgerKey>(
        &self,
        _key: &K,
        session_id: &str,
        model_id: &str,
        billing: Option<Billing>,
        usage: Option<&TokenUsage>,
    ) {
        if let Err(e) = self.try_record_usage(session_id, model_id, billing, usage) {
            tracing::warn!("usage of '{model_id}' not recorded for session {session_id}: {e:#}");
        }
    }

    fn try_record_usage(
        &self,
        session_id: &str,
        model_id: &str,
        billing: Option<Billing>,
        usage: Option<&TokenUsage>,
    ) -> anyhow::Result<()> {
        if !valid(session_id) {
            return Ok(());
        }
        // The store's own lock, held also by `remove_session`: a read-modify-
        // write is never half-way when the session goes.
        let _held = self.usage_lock.lock().unwrap_or_else(|e| e.into_inner());
        let dir = self.session_dir(session_id);
        if !dir.is_dir() {
            return Ok(());
        }
        let path = dir.join(FILE);
        let mut ledger = match read_ledger(&path) {
            Ok(ledger) => ledger,
            Err(why) => {
                set_aside(&path, &why);
                SessionUsage::default()
            }
        };
        let tally = ledger.models.entry(model_id.to_string()).or_default();
        tally.add(usage);
        tally.billing = billing;
        write_atomic(&path, &serde_json::to_vec(&ledger)?)?;
        Ok(())
    }

    /// What the session's models used so far; empty when none ran (or the
    /// ledger cannot be read — the next write sets it aside).
    pub fn session_usage(&self, session_id: &str) -> SessionUsage {
        if !valid(session_id) {
            return SessionUsage::default();
        }
        read_ledger(&self.session_dir(session_id).join(FILE)).unwrap_or_default()
    }
}

/// A missing file is an empty ledger; anything unreadable is an error.
fn read_ledger(path: &std::path::Path) -> Result<SessionUsage, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SessionUsage::default()),
        Err(e) => Err(e.to_string()),
    }
}

/// An unreadable ledger is kept under a new name and logged, never written
/// over: what it held may be recovered by hand.
fn set_aside(path: &std::path::Path, why: &str) {
    let aside = path.with_extension(format!("json.corrupt-{}", chrono::Utc::now().timestamp()));
    match std::fs::rename(path, &aside) {
        Ok(()) => tracing::warn!(
            "usage ledger unreadable ({why}); kept as {}",
            aside.display()
        ),
        Err(e) => {
            tracing::warn!("usage ledger unreadable ({why}) and not set aside ({e}); replaced")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestKey;
    impl LedgerKey for TestKey {}

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
        store.record_usage(
            &TestKey,
            "s1",
            "sonnet",
            None,
            Some(&call(1_000, 600, 100, 50)),
        );
        store.record_usage(&TestKey, "s1", "sonnet", None, Some(&call(2_000, 0, 0, 70)));
        store.record_usage(&TestKey, "s1", "gemini", None, Some(&call(10, 0, 0, 5)));
        store.record_usage(&TestKey, "s1", "gemini", None, None);
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
        store.record_usage(&TestKey, "no-such", "m", None, Some(&call(1, 0, 0, 1)));
        store.record_usage(&TestKey, "../escape", "m", None, Some(&call(1, 0, 0, 1)));
        assert!(!dir.path().join("no-such").exists());
        assert!(!dir.path().parent().unwrap().join("escape").exists());
    }

    /// Finding 9: a ledger that cannot be read is set aside under a new name
    /// and a fresh one started — never silently written over.
    #[test]
    fn an_unreadable_ledger_is_set_aside_not_written_over() {
        let (dir, store) = store();
        let path = dir.path().join("s1").join("usage.json");
        std::fs::write(&path, "{ not json").unwrap();
        // Reading it for the page says nothing, and touches nothing.
        assert!(store.session_usage("s1").is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
        store.record_usage(&TestKey, "s1", "m", None, Some(&call(10, 0, 0, 1)));
        assert_eq!(
            store.session_usage("s1").models["m"].calls,
            1,
            "a fresh ledger started"
        );
        let aside: Vec<_> = std::fs::read_dir(dir.path().join("s1"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with("usage.json.corrupt-"))
            .collect();
        assert_eq!(aside.len(), 1, "the unreadable file is kept: {aside:?}");
        let kept = std::fs::read_to_string(dir.path().join("s1").join(&aside[0])).unwrap();
        assert_eq!(kept, "{ not json");
    }

    /// Finding 8: a session removed while calls are being written is not an
    /// error, and its folder does not come back.
    #[test]
    fn a_session_removed_mid_write_stays_removed() {
        let (dir, store) = store();
        let store = std::sync::Arc::new(store);
        let writers: Vec<_> = (0..4)
            .map(|_| {
                let store = store.clone();
                std::thread::spawn(move || {
                    for _ in 0..50 {
                        store.record_usage(&TestKey, "s1", "m", None, Some(&call(10, 0, 0, 1)));
                    }
                })
            })
            .collect();
        std::thread::sleep(std::time::Duration::from_millis(2));
        store.remove_session("s1").unwrap();
        writers.into_iter().for_each(|t| t.join().unwrap());
        assert!(
            !dir.path().join("s1").exists(),
            "the removed session came back"
        );
    }

    /// Two stores on one folder (the engine and a test) never share a
    /// process-wide lock: the lock is the store's own.
    #[test]
    fn the_lock_is_the_stores_own() {
        let src = include_str!("session_usage.rs");
        assert!(!src.contains(concat!("static ", "WRITING")));
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
                        store.record_usage(&TestKey, "s1", "m", None, Some(&call(10, 0, 0, 1)));
                    }
                })
            })
            .collect();
        threads.into_iter().for_each(|t| t.join().unwrap());
        assert_eq!(store.session_usage("s1").models["m"].calls, 80);
    }
}

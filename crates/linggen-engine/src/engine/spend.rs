//! What a session's model calls used, noted where each call ends. One
//! function (`note_call`) feeds both tallies: the turn's `RunUsage` (a
//! mission run records it) and the session's ledger (`state_fs::session_usage`,
//! what the top bar and `agent_status` read). Written through per call, not
//! at the turn's end: a stopped turn has still spent what it spent.

use crate::engine::agent::AgentManager;
use crate::engine::AgentEngine;
use crate::provider::models::{SessionSpend, TokenUsage};

impl AgentManager {
    /// What the session's model calls used and, for the models with a price,
    /// cost — the one answer the chat's top bar and `agent_status` both
    /// give. `None` when no call has been noted (a Claude SDK session never
    /// is: its cost figure is the host's own).
    pub async fn session_spend(&self, session_id: &str) -> Option<SessionSpend> {
        let ledger = self.global_sessions.session_usage(session_id);
        if ledger.is_empty() {
            return None;
        }
        let models = self.models.read().await.clone();
        Some(models.spend_of(&ledger))
    }
}

impl AgentEngine {
    /// A model call ended (or its summary did): `usage` is what the provider
    /// reported, `None` when it reported nothing.
    pub(crate) fn note_call(&mut self, model_id: &str, usage: Option<&TokenUsage>) {
        self.run_usage.add(model_id, usage);
        let (Some(manager), Some(session_id)) = (self.tools.get_manager(), &self.session_id) else {
            return;
        };
        manager
            .global_sessions
            .record_usage(session_id, model_id, usage);
    }
}

#[cfg(test)]
mod lock {
    use std::path::{Path, PathBuf};

    fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// The ledger has one writer per kind of call: the engine's `note_call`
    /// and the next-prompt fork. A third caller fails here until it is
    /// looked at and listed — and the sum for a session stays one store.
    #[test]
    fn the_session_ledger_has_listed_writers_only() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        rust_files(&root.join("src"), &mut files);
        rust_files(&root.join("../../src"), &mut files);
        let mut found: Vec<(String, usize)> = files
            .iter()
            .filter(|f| !f.ends_with("state_fs/session_usage.rs"))
            .filter_map(|f| {
                // Whitespace out: a call split over lines is still a call.
                let text: String = std::fs::read_to_string(f)
                    .ok()?
                    .split_whitespace()
                    .collect();
                let n = text
                    .matches(concat!("global_sessions.", "record_usage("))
                    .count();
                (n > 0).then(|| (f.file_name().unwrap().to_string_lossy().to_string(), n))
            })
            .collect();
        found.sort();
        let spend = ("spend.rs".to_string(), 1);
        let fork = ("mod.rs".to_string(), 1);
        assert_eq!(
            found,
            vec![fork, spend],
            "a new writer of the session ledger: {found:?}"
        );
    }
}

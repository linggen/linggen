//! What a session's model calls used, noted where each call ends. One
//! function (`note_call`) feeds both tallies: the turn's `RunUsage` (a
//! mission run records it) and the session's ledger (`state_fs::session_usage`,
//! what the top bar and `agent_status` read). Written through per call, not
//! at the turn's end: a stopped turn has still spent what it spent.

use crate::engine::agent::AgentManager;
use crate::engine::AgentEngine;
use crate::provider::models::{spend_of, CallRecord, ModelManager, SessionSpend, TokenUsage};

impl AgentManager {
    /// What the session's model calls used and, for the models with a price,
    /// cost — the one answer the chat's top bar and `agent_status` both
    /// give. `None` when no call has been noted (a Claude SDK session never
    /// is: its cost figure is the host's own).
    pub fn session_spend(&self, session_id: &str) -> Option<SessionSpend> {
        let ledger = self.global_sessions.session_usage(session_id);
        if ledger.is_empty() {
            return None;
        }
        Some(spend_of(&ledger))
    }
}

/// What one model call's provider counted, kept from the moment its stream
/// opens — whether the call then finishes, is stopped, fails half-way or is
/// dropped. The provider bills what it counted either way.
#[derive(Debug, Default)]
pub struct CallSpend {
    opened: bool,
    usage: Option<TokenUsage>,
}

impl CallSpend {
    /// The provider accepted the request: from here the call may be billed.
    pub fn opened(&mut self) {
        self.opened = true;
    }

    /// A usage frame: Anthropic counts the prompt at the start and the
    /// output at the end, so frames merge.
    pub fn report(&mut self, usage: TokenUsage) {
        self.usage = Some(match self.usage.take() {
            Some(earlier) => earlier.merged(usage),
            None => usage,
        });
    }

    pub fn usage(&self) -> Option<&TokenUsage> {
        self.usage.as_ref()
    }

    pub fn is_opened(&self) -> bool {
        self.opened
    }
}

/// The key `SessionStore::record_usage` asks for. Its field is private to this
/// module, so only code here can make one; every model call (an engine's, a
/// summary, the next-prompt fork) is recorded through `record_call` below.
/// That makes a second writer a deliberate act — it must implement
/// `LedgerKey` for a type of its own, since the trait is public — not a
/// proof there is none: see the tripwire test at the end of this file.
pub struct Seal(());

impl crate::state_fs::session_usage::LedgerKey for Seal {}

/// Put what a call used into the session's ledger. A call that never opened
/// (refused, unreachable) cost nothing and is not a call.
fn record_call(
    manager: &AgentManager,
    models: &ModelManager,
    session_id: &str,
    model_id: &str,
    spent: &CallSpend,
) {
    if !spent.is_opened() {
        return;
    }
    // The call's class and cost are worked out now, from the config as it
    // is now, and kept: nothing later re-derives them.
    let rec = CallRecord::of(models.config_of(model_id), spent.usage());
    manager
        .global_sessions
        .record_usage(&Seal(()), session_id, model_id, &rec, spent.usage());
}

/// A summary of the context (`/compact`, outside any engine) is a call too.
pub fn note_summary(
    manager: &AgentManager,
    models: &ModelManager,
    session_id: &str,
    model_id: &str,
    spent: &CallSpend,
) {
    record_call(manager, models, session_id, model_id, spent);
}

/// The next-prompt fork's spend, put in the ledger when the fork is dropped
/// — at its end, or when the person types again and the next turn aborts it.
pub(crate) struct ForkSpend {
    pub manager: std::sync::Arc<AgentManager>,
    pub models: std::sync::Arc<ModelManager>,
    pub session_id: Option<String>,
    pub model_id: String,
    pub call: CallSpend,
}

impl Drop for ForkSpend {
    fn drop(&mut self) {
        if let Some(sid) = self.session_id.as_deref() {
            record_call(&self.manager, &self.models, sid, &self.model_id, &self.call);
        }
    }
}

impl AgentEngine {
    /// A model call ended, however it ended: `spent` is what its provider
    /// counted.
    pub(crate) fn note_call(&mut self, model_id: &str, spent: &CallSpend) {
        if !spent.is_opened() {
            return;
        }
        self.run_usage.add(model_id, spent.usage());
        let (Some(manager), Some(session_id)) = (self.tools.get_manager(), &self.session_id) else {
            return;
        };
        record_call(&manager, &self.model_manager, session_id, model_id, spent);
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

    /// A TRIPWIRE for the plain forms of a second writer, not a proof. It
    /// reads source text and counts: the ledger's write call as it is spelled
    /// today (reached through the manager's session store, whitespace ignored)
    /// in every file, the key being made, an
    /// `impl LedgerKey for`, and the ledger's file name outside its own
    /// module. A store held under another name, a macro, a renamed method or
    /// a write built another way passes it. It does not count `record_call`
    /// sites and says nothing about one call being counted twice — the
    /// engine tests (one call, one record) are for that.
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
        assert_eq!(
            found,
            vec![spend],
            "a new writer of the session ledger: {found:?}"
        );
        // The key is made in one place, the ledger's file named in one.
        let count = |needle: &str| -> Vec<String> {
            files
                .iter()
                .filter(|f| !f.ends_with("state_fs/session_usage.rs"))
                .filter(|f| std::fs::read_to_string(f).is_ok_and(|t| t.contains(needle)))
                .map(|f| f.file_name().unwrap().to_string_lossy().to_string())
                .collect()
        };
        let made = count(concat!("&Seal", "(())"));
        assert_eq!(
            made,
            vec!["spend.rs".to_string()],
            "a key to the ledger made elsewhere"
        );
        assert_eq!(
            count(concat!("\"usage", ".json\"")),
            Vec::<String>::new(),
            "the ledger's file named outside session_usage.rs"
        );
    }
}

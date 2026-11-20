//! What a session cost — the one answer the chat's top bar (the workspace
//! state route) and `agent_status` both give. A Claude SDK session's figure
//! is its host's own (`claude/usage.rs`); every model call the engine makes
//! is in the session's ledger (`AgentManager::session_spend`). A test below
//! fails when a second caller reads either of them.

use crate::server::ServerState;
use crate::provider::models::SessionSpend;
use crate::state_fs::sessions::SessionRuntime;
use serde::Serialize;

#[derive(Default, Serialize)]
pub(crate) struct SessionCost {
    /// The Claude SDK's own cost figure, for a Claude SDK session that has
    /// reported one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude_usage: Option<super::claude::usage::Usage>,
    /// What the engine's model calls used, and cost where a price is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<SessionSpend>,
}

/// A session id names one folder under the sessions folder.
fn names_a_folder(sid: &str) -> bool {
    !sid.is_empty() && !sid.contains('/') && !sid.contains("..")
}

pub(crate) fn of(state: &ServerState, sid: &str) -> SessionCost {
    if !names_a_folder(sid) {
        return SessionCost::default();
    }
    let sessions = &state.manager.global_sessions;
    let runtime = sessions
        .get_session_meta(sid)
        .ok()
        .flatten()
        .map(|meta| meta.runtime)
        .unwrap_or_default();
    let usage = state.manager.session_spend(sid);
    let claude_usage = match runtime {
        SessionRuntime::ClaudeAgentSdk => super::claude::usage::read(&sessions.session_dir(sid)),
        SessionRuntime::Linggen => None,
    };
    SessionCost {
        claude_usage,
        usage,
    }
}

#[cfg(test)]
mod tests {
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

    /// Lock: only this file asks what a session cost; the routes ask it.
    #[test]
    fn a_sessions_cost_is_read_only_here() {
        let server = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_files(&server, &mut files);
        let mut found = Vec::new();
        for path in files {
            let name = path.to_string_lossy();
            if name.ends_with("server/session_cost.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let code = text.split("#[cfg(test)]").next().unwrap_or("");
            for (n, line) in code.lines().enumerate() {
                if line.contains(".session_spend(") || line.contains("usage::read(") {
                    found.push(format!("{}:{}: {}", name, n + 1, line.trim()));
                }
            }
        }
        assert!(
            found.is_empty(),
            "ask server::session_cost::of instead:\n{}",
            found.join("\n")
        );
    }
}

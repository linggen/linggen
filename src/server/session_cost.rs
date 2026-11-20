//! What a session cost — the one answer the chat's top bar (the workspace
//! state route) and `agent_status` both give. A Claude SDK session's figure
//! is its host's own (`claude/usage.rs`); every model call the engine makes
//! is in the session's ledger (`AgentManager::session_spend`). A test below
//! fails when a second caller reads either of them.

use crate::provider::models::SessionSpend;
use crate::server::ServerState;
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

pub(crate) fn of(state: &ServerState, sid: &str) -> SessionCost {
    if !crate::state_fs::sessions::safe_session_id(sid) {
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
    /// Lock: only this file asks what a session cost; the routes ask it.
    #[test]
    fn a_sessions_cost_is_read_only_here() {
        let mut found = Vec::new();
        let mut scanned = false;
        for (rel, code) in crate::server::lock_scan::production_code(&["src"]) {
            let rel = rel.replace('\\', "/");
            scanned |= rel == "src/server/mcp_agent.rs";
            if rel == "src/server/session_cost.rs" {
                continue;
            }
            for (n, line) in code.lines().enumerate() {
                if line.contains(".session_spend(") || line.contains("usage::read(") {
                    found.push(format!("{rel}:{}: {}", n + 1, line.trim()));
                }
            }
        }
        assert!(scanned, "the scan did not reach the server");
        assert!(
            found.is_empty(),
            "ask server::session_cost::of instead:\n{}",
            found.join("\n")
        );
    }
}

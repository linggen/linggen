//! What a session cost — the one answer the chat's top bar (the workspace
//! state route) and `agent_status` both give: every model call the engine
//! makes is in the session's ledger (`AgentManager::session_spend`). A test
//! below fails when a second caller reads it.

use crate::provider::models::SessionSpend;
use crate::server::ServerState;
use serde::Serialize;

#[derive(Default, Serialize)]
pub(crate) struct SessionCost {
    /// What the engine's model calls used, and cost where a price is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<SessionSpend>,
}

pub(crate) fn of(state: &ServerState, sid: &str) -> SessionCost {
    if !crate::state_fs::sessions::safe_session_id(sid) {
        return SessionCost::default();
    }
    SessionCost {
        usage: state.manager.session_spend(sid),
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
                if line.contains(".session_spend(") {
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

//! Which agents are met, for the user's other devices and open pages
//! (`engine::agent::met`).
//!
//! An agent that appears only once met is latched the moment its declared
//! state is seen. Surfaces ask the gate directly; this loop is what notices
//! the state changing while nothing asks — it reads the waiting triggers on a
//! short tick, and whenever the answer changes it tells every open page
//! (`StateUpdated`) and republishes the retained `mac/agents` topic, the
//! mirror of `mac/tools`, which the phone reads for the same answer.

use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

use crate::engine::agent::met::AgentMet;
use crate::server::{api, ServerEvent, ServerState};

pub const TOPIC: &str = "mac";
pub const OP: &str = "agents";

/// The pace while any agent is still waiting to be met.
const WAITING_TICK: Duration = Duration::from_secs(3);
/// The pace once every declaring agent is met: only a changed spec can matter.
const SETTLED_TICK: Duration = Duration::from_secs(60);
/// How often the agents' declarations are read again while waiting.
const REDECLARE_EVERY: u32 = 20;

/// Keep `mac/agents` current for as long as the daemon runs.
pub async fn watch_loop(state: Arc<ServerState>) {
    let mut last: Option<Vec<AgentMet>> = None;
    let mut ticks: u32 = 0;
    loop {
        ticks = ticks.wrapping_add(1);
        let waiting = !state.manager.met.waiting().is_empty();
        if !waiting || ticks % REDECLARE_EVERY == 0 {
            state.manager.load_met_declarations().await;
        }
        state.manager.check_met_triggers().await;
        let now = state.manager.met.status();
        if last.as_ref() != Some(&now) {
            publish(&state, &now, last.is_some());
            last = Some(now);
        }
        let waiting = !state.manager.met.waiting().is_empty();
        tokio::time::sleep(if waiting { WAITING_TICK } else { SETTLED_TICK }).await;
    }
}

/// Retain it for whoever reads later, hand it to whoever is connected, and
/// tell the open pages to read the agent lists again (`changed`: not the
/// first publish after start).
fn publish(state: &Arc<ServerState>, agents: &[AgentMet], changed: bool) {
    let payload = payload(agents);
    api::topic::retain(TOPIC, OP, &payload);
    api::topic::publish_topic(state, TOPIC, OP, payload);
    if changed {
        let _ = state.events_tx.send(ServerEvent::StateUpdated);
    }
    tracing::info!(
        "[met] published {} agents that appear once met",
        agents.len()
    );
}

/// `{agents: [{id, present, met: {at, reason} | null}], host, published_at}`.
fn payload(agents: &[AgentMet]) -> serde_json::Value {
    json!({
        "agents": agents,
        "host": crate::perception::host_name(),
        "published_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::agent::met::{Met, MetReason};

    #[test]
    fn the_topic_carries_each_agent_and_whether_it_is_met() {
        let agents = vec![
            AgentMet {
                id: "her".into(),
                present: false,
                met: None,
            },
            AgentMet {
                id: "him".into(),
                present: true,
                met: Some(Met {
                    at: "2026-10-09T08:00:00Z".into(),
                    reason: MetReason::History,
                }),
            },
        ];
        let p = payload(&agents);
        assert_eq!(p["agents"][0]["id"], "her");
        assert_eq!(p["agents"][0]["present"], false);
        assert!(p["agents"][0]["met"].is_null());
        assert_eq!(p["agents"][1]["met"]["reason"], "history");
        assert_eq!(p["agents"][1]["met"]["at"], "2026-10-09T08:00:00Z");
        assert!(p["published_at"].is_string());
    }
}

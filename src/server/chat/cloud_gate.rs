//! The account gate for a skill that declares `cloud:` — at a turn's edges.
//! Before: signed in (`AUTH_REQUIRED:` otherwise) and the save pulled into
//! step. After: one last sync. These edges are the only turn-time pulls; the
//! meter and a mid-turn push live around each model call instead
//! (`engine/cloud_meter.rs`): a game sitting is one long turn.

use crate::account::cloud_save::{sync, target, SaveTarget};
use crate::engine::{AgentEngine, AgentOutcome};
use crate::server::api::skill_cloud::AUTH_REQUIRED;
use crate::server::ServerEvent;
use tokio::sync::broadcast;

/// Run the turn inside its skill's cloud gate, when the skill declares one.
/// Every other turn runs exactly as before.
pub(super) async fn run_gated(
    engine: &mut AgentEngine,
    session_id: Option<&str>,
    events_tx: &broadcast::Sender<ServerEvent>,
) -> anyhow::Result<AgentOutcome> {
    // Read before the loop, which clears the active skill when it ends.
    let Some(save) = cloud_save(engine) else {
        return engine.run_agent_loop(session_id).await;
    };
    if crate::account::resolve_token().is_none() {
        anyhow::bail!(AUTH_REQUIRED);
    }
    sync_logged(save.as_ref(), "before the turn", events_tx).await;
    let result = engine.run_agent_loop(session_id).await;
    sync_logged(save.as_ref(), "after the turn", events_tx).await;
    result
}

/// `Some(save?)` when the active skill declares a cloud at all.
fn cloud_save(engine: &AgentEngine) -> Option<Option<SaveTarget>> {
    let skill = engine.active_skill.as_ref()?;
    skill.cloud.as_ref()?;
    Some(target(skill))
}

async fn sync_logged(
    save: Option<&SaveTarget>,
    when: &str,
    events_tx: &broadcast::Sender<ServerEvent>,
) {
    let Some(save) = save else { return };
    match sync(save).await {
        Ok(done) => crate::server::api::skill_cloud::announce(events_tx, &save.skill, &done),
        Err(e) => tracing::warn!("save '{}' not synced {when}: {e:#}", save.skill),
    }
}

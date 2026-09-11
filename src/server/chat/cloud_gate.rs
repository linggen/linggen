//! The account gate for a skill that declares `cloud:` — at a turn's edges.
//! Before: signed in (`AUTH_REQUIRED:` otherwise) and the save pulled into
//! step. After: one last sync. The meter lives around each model call
//! instead (`engine/cloud_meter.rs`): a game sitting is one long turn.

use crate::account::cloud_save::{sync, target, SaveTarget};
use crate::engine::{AgentEngine, AgentOutcome};

/// Run the turn inside its skill's cloud gate, when the skill declares one.
/// Every other turn runs exactly as before.
pub(super) async fn run_gated(
    engine: &mut AgentEngine,
    session_id: Option<&str>,
) -> anyhow::Result<AgentOutcome> {
    // Read before the loop, which clears the active skill when it ends.
    let Some(save) = cloud_save(engine) else {
        return engine.run_agent_loop(session_id).await;
    };
    if crate::account::resolve_token().is_none() {
        anyhow::bail!("AUTH_REQUIRED: Sign in to linggen.dev to play.");
    }
    sync_logged(save.as_ref(), "before the turn").await;
    let result = engine.run_agent_loop(session_id).await;
    sync_logged(save.as_ref(), "after the turn").await;
    result
}

/// `Some(save?)` when the active skill declares a cloud at all.
fn cloud_save(engine: &AgentEngine) -> Option<Option<SaveTarget>> {
    let skill = engine.active_skill.as_ref()?;
    skill.cloud.as_ref()?;
    Some(target(skill))
}

async fn sync_logged(save: Option<&SaveTarget>, when: &str) {
    let Some(save) = save else { return };
    if let Err(e) = sync(save).await {
        tracing::warn!("save '{}' not synced {when}: {e:#}", save.skill);
    }
}

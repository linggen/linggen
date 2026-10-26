//! Asking the SDK whether a permission mode is available spawns a runner, so
//! each question is asked once at a time and its answer kept: a success for
//! the engine's lifetime, a failure briefly (a fixed model or login may
//! change it). The key carries everything the answer depends on.
use super::{resolve_model, runtime, trust, AGENT_ID};
use crate::server::ServerState;
use crate::state_fs::sessions::{ClaudePermissionMode, SessionMeta};
use anyhow::Result;
use std::time::{Duration, Instant};

pub(crate) type Outcome = (Instant, std::result::Result<(), String>);

const FAILURE_TTL: Duration = Duration::from_secs(60);

pub(super) fn key(session: &str, mode: ClaudePermissionMode, model: &str, trusted: bool) -> String {
    format!("{session}|{mode:?}|{model}|{trusted}")
}

/// A kept answer still worth giving.
pub(super) fn fresh(outcome: &Outcome, now: Instant) -> bool {
    outcome.1.is_ok() || now.duration_since(outcome.0) < FAILURE_TTL
}

pub(super) async fn probe(
    state: &ServerState,
    meta: &SessionMeta,
    mode: ClaudePermissionMode,
) -> Result<()> {
    let model = resolve_model(
        state,
        meta.member(AGENT_ID).and_then(|m| m.model.as_deref()),
    )
    .await?;
    let trusted = trust::known(state, &meta.id, &meta.cwd);
    let slot = state
        .claude
        .probes
        .lock()
        .await
        .entry(key(&meta.id, mode, &model.id, trusted))
        .or_default()
        .clone();
    // Holding the slot makes concurrent askers wait for one runner.
    let mut slot = slot.lock().await;
    if let Some(outcome) = slot.as_ref().filter(|o| fresh(o, Instant::now())) {
        return outcome.1.clone().map_err(anyhow::Error::msg);
    }
    let result = runtime::validate_mode(state, meta, mode, trusted)
        .await
        .map_err(|error| error.to_string());
    *slot = Some((Instant::now(), result.clone()));
    result.map_err(anyhow::Error::msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successes_are_kept_and_failures_expire() {
        let start = Instant::now();
        let ok: Outcome = (start, Ok(()));
        let failed: Outcome = (start, Err("unavailable".into()));
        assert!(fresh(&ok, start + Duration::from_secs(3600)));
        assert!(fresh(&failed, start + Duration::from_secs(1)));
        assert!(!fresh(&failed, start + FAILURE_TTL));
    }

    #[test]
    fn answers_differ_by_mode_model_and_trust() {
        let base = key("s", ClaudePermissionMode::Auto, "m", false);
        assert_ne!(base, key("s", ClaudePermissionMode::Default, "m", false));
        assert_ne!(base, key("s", ClaudePermissionMode::Auto, "n", false));
        assert_ne!(base, key("s", ClaudePermissionMode::Auto, "m", true));
    }
}

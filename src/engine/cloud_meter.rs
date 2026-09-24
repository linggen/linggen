//! A skill's cloud (`CloudConfig`) around every model call. A game sitting is
//! one long turn — narration, an AskUser answer, more narration — so a check
//! at the turn's start would never stop it, and a report at its end would
//! come too late. Before a call: a spent meter refuses it (`BUDGET_EMPTY:`).
//! After it: the call's tokens are reported and a change to the save is
//! pushed, in the background — never pulled: the tool call that follows may be
//! writing the save right now, and a pull would land under it. Pulls belong to
//! the turn's edges (`server/chat/cloud_gate.rs`). linggen.dev unreachable →
//! the call goes ahead: the meter is a pace, not a lock.

use crate::account::cloud::{meter_reading, meter_report};
use crate::account::cloud_save::{push, target};
use crate::engine::AgentEngine;
use crate::provider::models::TokenUsage;

fn meter_of(engine: &AgentEngine) -> Option<String> {
    engine.active_skill.as_ref()?.cloud.as_ref()?.meter.clone()
}

pub(crate) async fn before_call(engine: &AgentEngine) -> anyhow::Result<()> {
    let Some(meter) = meter_of(engine) else {
        return Ok(());
    };
    match meter_reading(&meter).await {
        Ok(r) if r.left == 0 => {
            anyhow::bail!("BUDGET_EMPTY: refill_at={}", r.refill_at.unwrap_or(0))
        }
        Ok(_) => Ok(()),
        Err(e) => {
            tracing::warn!("meter '{meter}' unreachable, playing on: {e:#}");
            Ok(())
        }
    }
}

pub(crate) fn after_call(engine: &AgentEngine, usage: Option<&TokenUsage>) {
    let Some(skill) = engine.active_skill.as_ref() else {
        return;
    };
    if skill.cloud.is_none() {
        return;
    }
    let meter = meter_of(engine);
    let tokens = usage.map(|u| u.metered()).unwrap_or(0);
    match usage {
        Some(u) => tracing::debug!(
            "meter: {tokens} tokens this call (prompt {:?}, cached {:?}, output {:?})",
            u.prompt_tokens,
            u.cached_tokens,
            u.completion_tokens
        ),
        None => tracing::debug!("meter: the provider reported no usage for this call"),
    }
    let save = target(skill);
    tokio::spawn(async move {
        if let (Some(meter), true) = (meter.as_deref(), tokens > 0) {
            if let Err(e) = meter_report(meter, tokens).await {
                tracing::warn!("meter '{meter}': {tokens} tokens not reported: {e:#}");
            }
        }
        if let Some(save) = save {
            if let Err(e) = push(&save).await {
                tracing::warn!("save '{}' not pushed: {e:#}", save.skill);
            }
        }
    });
}

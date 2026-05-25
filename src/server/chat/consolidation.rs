//! Every-N-turns memory consolidation trigger + tick.
//!
//! Fired from the post-turn seam in [`super::handler`] once a session has
//! completed a multiple of `consolidate_every_n_turns` turns. The cadence
//! is *derived* from chat history (see
//! [`crate::engine::memory::should_consolidate`]) — there is no persisted
//! counter, so it is per-session and restart-safe by construction.
//!
//! The work is split into two independently-invocable phases (one
//! `ling-mem` subagent call each, `agents/ling-mem.md`; tool = Bash only;
//! the task carries the absolute binary path for PATH-independent
//! resolvability — not a security lock, see the contract memory):
//!
//! 1. [`run_encode`] — write the recent exchange into the **episodic**
//!    table. Per-session, wake-time. ≈ hippocampal encoding.
//! 2. [`run_consolidate_evict`] — terminally promote/delete the *past-TTL*
//!    worklist the engine pre-selects, then a deterministic `ling-mem
//!    evict` backstop. **Global**, not session-scoped. ≈ sleep
//!    consolidation + synaptic downscaling.
//!
//! The engine owns TTL policy: it computes the absolute cutoff once,
//! selects the past-TTL worklist itself (binary stays policy-free), and
//! hands it to the subagent.
//!
//! The per-session tick is **encode-only** (wake-time). Consolidate +
//! evict is owned entirely by the built-in `dream` mission (daily cron +
//! turn-seam catch-up; see `missions::scheduler`), which calls
//! [`run_consolidate_evict`] directly. This is the Complementary
//! Learning Systems split: fast hippocampal encoding while awake, slow
//! neocortical consolidation + forgetting offline.
//! See `project_memory_recall_redesign` / `project_consolidator_contract`.

use super::ChatRunCtx;
use crate::agent_manager::AgentManager;
use crate::engine::{memory, AgentEngine};
use crate::server::ServerState;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// How many trailing user+assistant messages to hand the encoder as "the
/// recent exchange". Two per turn (user + assistant), so `2 * interval`
/// covers the window since the previous tick with headroom.
const TRANSCRIPT_TURN_MULTIPLIER: usize = 2;
/// Per-message content cap in the transcript snapshot — long tool dumps
/// don't belong in an encoding prompt. Mirrors `context.rs::summarize_span`.
const TRANSCRIPT_MSG_CHARS: usize = 2000;
/// Per-row content cap in the worklist block. Episodic facts are
/// one-liners; this bounds the prompt and contains an oversized or
/// instruction-laden row (the user's own data, but still untrusted text
/// flowing into a prompt).
const WORKLIST_CONTENT_CHARS: usize = 500;
/// Bound on a single deterministic `ling-mem` invocation.
const LING_MEM_TIMEOUT: Duration = Duration::from_secs(30);

/// Result of one per-session encode tick — how many rows were written to
/// episodic. Only logged. Consolidation runs separately as a mission;
/// it doesn't share this counter.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct TickOutcome {
    pub encoded: u32,
}

/// Inspect the just-completed turn and, if this session hit a
/// consolidation interval, spawn the tick off the user's turn. Reads
/// everything from the still-locked `engine` up front so the spawned task
/// never touches the engine lock.
///
/// No-ops (cheap, no spawn) when: not an owner session
/// (`include_memory == false` — consumer never consolidates the user's
/// biography), the session was created by a skill or mission scheduler
/// (their transcripts are task-bound and not about the user — the
/// mission→user promotion in `promote_mission_session_to_user` keeps
/// the human-takeover case eligible), the turn count isn't a positive
/// multiple of the interval, or there is no real session id to key the
/// overlap guard.
pub(super) fn maybe_fire_consolidation(ctx: &ChatRunCtx, engine: &AgentEngine) {
    if !engine.prompt_profile.include_memory {
        return;
    }
    let interval = engine.cfg.consolidate_every_n_turns;
    if !memory::should_consolidate(&engine.chat_history, interval) {
        return;
    }
    let Some(session_id) = ctx.session_id.clone() else {
        return;
    };
    let creator = ctx
        .state
        .manager
        .global_sessions
        .get_session_meta(&session_id)
        .ok()
        .flatten()
        .map(|m| m.creator)
        .unwrap_or_else(|| "user".to_string());
    if creator != "user" {
        return;
    }

    let recent_transcript = snapshot_recent_exchange(engine, interval);
    let state = ctx.state.clone();
    let manager = ctx.manager.clone();
    let agent_id = ctx.agent_id.clone();
    let ws_root = ctx.root.clone();

    tokio::spawn(async move {
        // Per-session overlap guard. Acquired *inside* the task so two
        // rapidly-qualifying turns can't both run: the second observes the
        // id already present and bows out. Mirrors the mission scheduler's
        // per-mission `running` flag.
        {
            let mut active = state.consolidation_active.lock().await;
            if !active.insert(session_id.clone()) {
                tracing::debug!(
                    session_id = %session_id,
                    "memory consolidation already in flight; skipping this tick"
                );
                return;
            }
        }

        let result = run_consolidation_tick(
            &state,
            &manager,
            &session_id,
            &agent_id,
            &ws_root,
            recent_transcript,
        )
        .await;

        match result {
            Ok(o) if o.encoded > 0 => tracing::info!(
                session_id = %session_id,
                encoded = o.encoded,
                "memory encode tick: wrote episodic rows"
            ),
            Ok(_) => tracing::debug!(
                session_id = %session_id,
                "memory encode tick: nothing durable"
            ),
            Err(e) => tracing::warn!(
                session_id = %session_id,
                error = %e,
                "memory encode tick failed"
            ),
        }

        // Best-effort release. Errors (Err arm above) still reach here —
        // only a genuine Rust panic in the tick skips it, since the task
        // future isn't awaited. Consequence: that session gets *zero*
        // further consolidation ticks until the daemon restarts (which
        // clears the set), not just a one-tick miss. Accepted — matches
        // the mission scheduler's guard robustness; the fix for a
        // panicking tick is the tick, not masking it here.
        state.consolidation_active.lock().await.remove(&session_id);
    });
}

/// Flatten the trailing user+assistant messages into `[role] text` lines
/// for the encoder. Format mirrors `context.rs::summarize_span`.
fn snapshot_recent_exchange(engine: &AgentEngine, interval: usize) -> String {
    let want = interval.saturating_mul(TRANSCRIPT_TURN_MULTIPLIER).max(2);
    let mut lines: Vec<String> = engine
        .chat_history
        .iter()
        .rev()
        .filter(|m| m.role == "user" || m.role == "assistant")
        .take(want)
        .map(|m| {
            let body: String = m.content.chars().take(TRANSCRIPT_MSG_CHARS).collect();
            let ellipsis = if m.content.chars().count() > TRANSCRIPT_MSG_CHARS {
                "…"
            } else {
                ""
            };
            format!("[{}] {}{}", m.role, body, ellipsis)
        })
        .collect();
    lines.reverse();
    lines.join("\n")
}

/// The per-session tick: **encode only** (wake-time). Consolidate +
/// evict is owned by the `dream` mission (`missions::scheduler`), not the
/// per-session path — see the module doc. Promote/delete counts are not
/// produced here.
async fn run_consolidation_tick(
    state: &Arc<ServerState>,
    manager: &Arc<AgentManager>,
    session_id: &str,
    agent_id: &str,
    ws_root: &PathBuf,
    recent_transcript: String,
) -> anyhow::Result<TickOutcome> {
    let encoded = run_encode(
        state,
        manager,
        session_id,
        agent_id,
        ws_root,
        &recent_transcript,
    )
    .await?;

    Ok(TickOutcome { encoded })
}

/// **Encode phase.** Write the recent exchange into episodic. Per-session,
/// wake-time. Empty transcript → no subagent spawn, returns 0.
async fn run_encode(
    state: &Arc<ServerState>,
    manager: &Arc<AgentManager>,
    session_id: &str,
    agent_id: &str,
    ws_root: &PathBuf,
    recent_transcript: &str,
) -> anyhow::Result<u32> {
    if recent_transcript.trim().is_empty() {
        return Ok(0);
    }
    let _ = state; // memory daemon is reached through Memory_* tools now
    let task = build_encode_task(session_id, recent_transcript);
    // Encoder runs post-turn with the user reachable — keeps AskUser
    // via the standard `ling-mem` spec for contradiction reconcile.
    let summary = run_ling_mem_subagent(state, manager, ws_root, agent_id, session_id, task)
        .await
        .map_err(|e| e.context("ling-mem encode subagent"))?;
    parse_encoded(&summary)
}

/// **Consolidate + evict phase.** Terminally promote/delete the past-TTL
/// Resolve the `ling-mem` binary + data dir. `ling-mem` is installed by
/// linggen itself (see `linggensite/public/install.sh`) — alongside `ling`
/// in `/usr/local/bin` (preferred) or `~/.local/bin` (fallback). The
/// resolver also accepts a `$PATH` match for hand-installed copies. The
/// task prompt directs the subagent to this absolute path so commands
/// resolve regardless of `PATH` (resolvability, not a security lock).
#[allow(dead_code)]
async fn resolve_ling_mem(_state: &Arc<ServerState>) -> (PathBuf, PathBuf) {
    let bin = crate::engine::capability_tools::resolve_binary(None, "ling-mem");
    let data_dir = crate::paths::linggen_home().to_path_buf();
    (bin, data_dir)
}

/// Run one `ling-mem` subagent phase via the standard delegation path —
/// same lifecycle every subagent uses: run-record tracking, cancellation,
/// and the SubagentSpawned/SubagentResult/AgentStatus events the widget
/// consumes. `parent_interactive=false` keeps it unattended (no prompts →
/// no deadlock); the `ling-mem` agent spec pins `tools: ["Bash"]`, and
/// the task carries the absolute binary path so commands resolve without
/// relying on PATH.
async fn run_ling_mem_subagent(
    state: &Arc<ServerState>,
    manager: &Arc<AgentManager>,
    ws_root: &PathBuf,
    agent_id: &str,
    session_id: &str,
    task: String,
) -> anyhow::Result<String> {
    // The encoder runs post-turn with the user reachable, so wire an
    // AskUserBridge through ServerState. The widget surfaces in the
    // SubagentPane tab keyed to the encoder's agent_id. Subagent AskUser
    // already times out at 5 min (tools/mod.rs::ask_user) — no deadlock
    // risk. (The dream mission no longer uses this function — it runs as
    // a generic mission with no AskUser by mission policy.)
    let ask_user_bridge = Some(Arc::new(crate::engine::tools::AskUserBridge {
        events_tx: state.events_tx.clone(),
        pending: state.pending_ask_user.clone(),
        session_id: Some(session_id.to_string()),
    }));

    let result = crate::engine::tools::run_delegation(
        manager.clone(),
        ws_root.clone(),
        agent_id.to_string(), // caller_id
        "ling-mem".to_string(),
        task,
        None, // parent_run_id
        0,    // delegation_depth
        1,    // max_delegation_depth — never re-delegates
        ask_user_bridge,
        Some(session_id.to_string()),
        None,       // parent_policy — spec tools restrict it
        Vec::new(), // parent_path_modes
        false,      // parent_interactive
    )
    .await?;

    use crate::engine::tools::ToolResult;
    match result {
        ToolResult::Success(text) => Ok(text),
        // The Bash-only non-interactive agent should always finish with
        // AgentOutcome::None → Success. Anything else (an unexpected
        // plan/done outcome) is a bug worth seeing in the log.
        ToolResult::AgentOutcome(o) => {
            anyhow::bail!("subagent ended with outcome {o:?}, not a result")
        }
        other => anyhow::bail!("subagent returned {other:?}"),
    }
}

/// ENCODE-phase task. The subagent talks to memory through the
/// `Memory_query` / `Memory_write` capability tools (HTTP-dispatched to
/// the active memory daemon), so the task no longer carries a binary
/// path or `--data-dir`.
fn build_encode_task(session_id: &str, recent_transcript: &str) -> String {
    let today = Utc::now().format("%Y-%m-%d");
    let turn_count = recent_transcript
        .lines()
        .filter(|l| l.starts_with("[user]"))
        .count();
    // Lead with a friendly one-liner so the SubagentPane shows a
    // readable summary above the technical instructions; LLM still
    // sees the full spec below.
    format!(
        "📝 Review the last {n} turn{plural} of the user's chat in main \
         and decide which durable signal — facts, preferences, decisions, \
         reusable gotchas — is worth saving to episodic memory.\n\
         \n\
         ---\n\
         Phase: ENCODE. Session `{session}`. Today is {today}.\n\
         \n\
         Encode this recent exchange via `Memory_write({{verb: \"add\", \
         host: \"linggen\", …}})`. Follow `[memory_protocol]` in your \
         system prompt for the read-before-write rule, the AskUser \
         contradiction shape, and tier selection — **pick the tier \
         (core / semantic / episodic) per row by confidence**, not by a \
         hardcoded default. Apply the exclusion filters and the \
         usefulness bar from your instructions. Date-stamp ages relative \
         to {today}.\n\
         <recent-exchange>\n{transcript}\n</recent-exchange>\n\
         \n\
         Then emit your ENCODED status block (count line plus one \
         bullet per encoded row, see your agent instructions) and stop.",
        n = turn_count,
        plural = if turn_count == 1 { "" } else { "s" },
        session = session_id,
        today = today,
        transcript = recent_transcript,
    )
}

/// Parse the ENCODE contract line: `ENCODED encoded=<n>` or
/// `ENCODE_FAILED <reason>`.
fn parse_encoded(summary: &str) -> anyhow::Result<u32> {
    let line = summary
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with("ENCODED") || l.starts_with("ENCODE_FAILED"))
        .ok_or_else(|| anyhow::anyhow!("subagent emitted no ENCODED/ENCODE_FAILED line"))?;

    if let Some(reason) = line.strip_prefix("ENCODE_FAILED") {
        anyhow::bail!("encode reported failure:{}", reason);
    }
    Ok(field(line, "encoded"))
}

/// Read `key=<u32>` out of a status line; missing/garbage → 0.
fn field(line: &str, key: &str) -> u32 {
    line.split_whitespace()
        .filter_map(|tok| tok.split_once('='))
        .find(|(k, _)| *k == key)
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_encoded_reads_count() {
        assert_eq!(parse_encoded("chatter\nENCODED encoded=3").unwrap(), 3);
        assert_eq!(parse_encoded("ENCODED encoded=0").unwrap(), 0);
    }

    #[test]
    fn parse_encoded_failure_and_missing_are_err() {
        assert!(parse_encoded("ENCODE_FAILED episodic unreadable").is_err());
        assert!(parse_encoded("did stuff, forgot the line").is_err());
    }

}

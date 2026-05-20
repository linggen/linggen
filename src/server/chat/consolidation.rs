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
/// episodic. Only logged. (Promote/delete counts belong to the `dream`
/// mission's consolidate path, which returns its own tuple.)
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct TickOutcome {
    pub encoded: u32,
}

/// Minimal projection of an episodic `ling-mem` row — only the fields the
/// past-TTL filter and the consolidate worklist need. Unknown fields
/// (notably the embedding `vector`) are ignored by serde.
#[derive(Debug, Deserialize)]
struct EpisodicRow {
    id: String,
    content: String,
    #[serde(rename = "type")]
    fact_type: String,
    #[serde(default)]
    contexts: Vec<String>,
    // RFC-3339 strings — kept as `String` so we don't need chrono's
    // `serde` feature (not enabled in this crate). Parsed in `decay_ts`.
    created_at: String,
    #[serde(default)]
    updated_at: Option<String>,
}

impl EpisodicRow {
    /// The decay clock — `updated_at` if the row was ever touched, else
    /// `created_at` (a touch resets retention, `memory-spec.md` §2). Must
    /// match `linggen-memory`'s `evict` clock exactly. An unparseable
    /// stamp is treated as epoch (always past-TTL) so a malformed row is
    /// surfaced to the consolidator rather than silently retained forever.
    fn decay_ts(&self) -> DateTime<Utc> {
        let raw = self.updated_at.as_deref().unwrap_or(&self.created_at);
        DateTime::parse_from_rfc3339(raw)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
    }
}

/// Inspect the just-completed turn and, if this session hit a
/// consolidation interval, spawn the tick off the user's turn. Reads
/// everything from the still-locked `engine` up front so the spawned task
/// never touches the engine lock.
///
/// No-ops (cheap, no spawn) when: not an owner session
/// (`include_memory == false` — consumer/mission never consolidate the
/// user's biography), the turn count isn't a positive multiple of the
/// interval, or there is no real session id to key the overlap guard.
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
    let summary = run_ling_mem_subagent(manager, ws_root, agent_id, session_id, task)
        .await
        .map_err(|e| e.context("ling-mem encode subagent"))?;
    parse_encoded(&summary)
}

/// **Consolidate + evict phase.** Terminally promote/delete the past-TTL
/// worklist, then a deterministic evict backstop. **Global** (the
/// worklist is every past-TTL episodic row, not session-scoped);
/// `session_id` keys the run record / overlap guard / logs only.
///
/// Empty worklist → nothing is past-TTL, so there is also nothing to
/// evict: no subagent spawn, returns `(0, 0, 0)`.
pub(crate) async fn run_consolidate_evict(
    state: &Arc<ServerState>,
    manager: &Arc<AgentManager>,
    session_id: &str,
    agent_id: &str,
    ws_root: &PathBuf,
    episodic_ttl_days: u64,
) -> anyhow::Result<(u32, u32)> {
    // The engine owns TTL policy: one absolute cutoff drives both the
    // worklist selection and the evict backstop.
    let cutoff = Utc::now() - ChronoDuration::days(episodic_ttl_days as i64);
    // Worklist selection + the deterministic evict backstop still talk
    // to the binary (engine-direct, no subagent), so we keep
    // resolve_ling_mem for those two callers.
    let (bin, data_dir) = resolve_ling_mem(state).await;

    let worklist = select_past_ttl_worklist(&bin, &data_dir, cutoff)
        .await
        .map_err(|e| e.context("listing episodic rows for consolidation"))?;
    if worklist.is_empty() {
        return Ok((0, 0));
    }

    let task = build_consolidate_task(&cutoff, &worklist);
    let summary = run_ling_mem_subagent(manager, ws_root, agent_id, session_id, task)
        .await
        .map_err(|e| e.context("ling-mem consolidate subagent"))?;
    let (promoted, deleted) = parse_consolidated(&summary)?;

    // Deterministic backstop: evict whatever past-TTL rows the subagent
    // didn't terminally handle. Same cutoff. Failures here are non-fatal —
    // the rows simply age out on a later run.
    if let Err(e) = run_ling_mem(&bin, &data_dir, &["evict", "--before", &cutoff.to_rfc3339()]).await
    {
        tracing::warn!(session_id = %session_id, error = %e, "evict backstop failed (non-fatal)");
    }

    Ok((promoted, deleted))
}

/// Resolve the `ling-mem` binary + data dir. `ling-mem` lives at
/// `$SKILL_DIR/bin/ling-mem` (memory skill install) with a `$PATH`
/// fallback; the task prompt directs the subagent to this absolute path
/// so commands resolve regardless of PATH (resolvability, not a security
/// lock — see the module/contract docs on the boundary).
async fn resolve_ling_mem(state: &Arc<ServerState>) -> (PathBuf, PathBuf) {
    let skill_dir = state
        .skill_manager
        .active_provider("memory")
        .await
        .and_then(|s| s.skill_dir);
    let bin = crate::engine::capability_tools::resolve_binary(skill_dir.as_deref(), "ling-mem");
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
    manager: &Arc<AgentManager>,
    ws_root: &PathBuf,
    agent_id: &str,
    session_id: &str,
    task: String,
) -> anyhow::Result<String> {
    let result = crate::engine::tools::run_delegation(
        manager.clone(),
        ws_root.clone(),
        agent_id.to_string(), // caller_id
        "ling-mem".to_string(),
        task,
        None, // parent_run_id
        0,    // delegation_depth
        1,    // max_delegation_depth — never re-delegates
        None, // ask_user_bridge — unattended
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

/// `ling-mem list --episodic --format json`, then keep only rows whose
/// decay clock is older than `cutoff`. The binary stays policy-free; the
/// engine applies the TTL.
async fn select_past_ttl_worklist(
    bin: &Path,
    data_dir: &Path,
    cutoff: DateTime<Utc>,
) -> anyhow::Result<Vec<EpisodicRow>> {
    let stdout = run_ling_mem(bin, data_dir, &["list", "--episodic", "--format", "json"]).await?;
    let rows = parse_episodic_rows(&stdout)?;
    Ok(rows.into_iter().filter(|r| r.decay_ts() < cutoff).collect())
}

/// Accept either a JSON array or newline-delimited JSON objects (the CLI
/// has used both shapes for list output).
fn parse_episodic_rows(stdout: &str) -> anyhow::Result<Vec<EpisodicRow>> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if let Ok(rows) = serde_json::from_str::<Vec<EpisodicRow>>(trimmed) {
        return Ok(rows);
    }
    let mut rows = Vec::new();
    for line in trimmed.lines().filter(|l| !l.trim().is_empty()) {
        rows.push(
            serde_json::from_str::<EpisodicRow>(line)
                .map_err(|e| anyhow::anyhow!("episodic row parse failed: {e}; line: {line}"))?,
        );
    }
    Ok(rows)
}

/// Run one `ling-mem` subcommand deterministically (no LLM). Always
/// pins `--data-dir` so it never depends on env or cwd.
async fn run_ling_mem(bin: &Path, data_dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = tokio::time::timeout(
        LING_MEM_TIMEOUT,
        tokio::process::Command::new(bin)
            .arg("--data-dir")
            .arg(data_dir)
            .args(args)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("`ling-mem {}` timed out", args.join(" ")))?
    .map_err(|e| anyhow::anyhow!("spawning `ling-mem {}`: {e}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "`ling-mem {}` exited {}: {}",
            args.join(" "),
            output.status,
            stderr.trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// ENCODE-phase task. The subagent talks to memory through the
/// `Memory_query` / `Memory_write` capability tools (HTTP-dispatched to
/// the active memory daemon), so the task no longer carries a binary
/// path or `--data-dir`.
fn build_encode_task(session_id: &str, recent_transcript: &str) -> String {
    let today = Utc::now().format("%Y-%m-%d");
    format!(
        "Phase: ENCODE. Session `{session}`. Today is {today}.\n\
         \n\
         Encode this recent exchange into episodic via \
         `Memory_write({{verb: \"add\", episodic: true, host: \"linggen\", \
         …}})`. Apply the exclusion filters and the usefulness bar from \
         your instructions. Date-stamp ages relative to {today}.\n\
         <recent-exchange>\n{transcript}\n</recent-exchange>\n\
         \n\
         Then emit your single ENCODED status line and stop.",
        session = session_id,
        today = today,
        transcript = recent_transcript,
    )
}

/// CONSOLIDATE-phase task. The engine already selected the past-TTL
/// worklist (the binary stays policy-free for selection); the subagent
/// only makes the terminal promote/delete call per row via `Memory_*`.
fn build_consolidate_task(cutoff: &DateTime<Utc>, worklist: &[EpisodicRow]) -> String {
    let worklist_block = worklist
        .iter()
        .map(|r| {
            let ctx = if r.contexts.is_empty() {
                String::new()
            } else {
                format!(" contexts={}", r.contexts.join(","))
            };
            let content: String = r.content.chars().take(WORKLIST_CONTENT_CHARS).collect();
            let ellipsis = if r.content.chars().count() > WORKLIST_CONTENT_CHARS {
                "…"
            } else {
                ""
            };
            format!(
                "- id={} type={}{}\n  content: {}{}",
                r.id, r.fact_type, ctx, content, ellipsis
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "Phase: CONSOLIDATE.\n\
         \n\
         These episodic rows are older than the {cutoff} TTL cutoff. The \
         engine already selected them — do NOT list episodic yourself. \
         For EACH: promote (write to semantic with `Memory_write({{verb: \
         \"add\", host: \"linggen\", …}})`, then \
         `Memory_write({{verb: \"delete\", episodic: true, id: <id>}})`) \
         or delete only (`Memory_write({{verb: \"delete\", episodic: true, \
         id: <id>}})`).\n\
         <worklist>\n{worklist_block}\n</worklist>\n\
         \n\
         Then emit your single CONSOLIDATED status line and stop.",
        cutoff = cutoff.to_rfc3339(),
        worklist_block = worklist_block,
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

/// Parse the CONSOLIDATE contract line:
/// `CONSOLIDATED promoted=<n> deleted=<n>` or `CONSOLIDATE_FAILED <reason>`.
fn parse_consolidated(summary: &str) -> anyhow::Result<(u32, u32)> {
    let line = summary
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with("CONSOLIDATED") || l.starts_with("CONSOLIDATE_FAILED"))
        .ok_or_else(|| {
            anyhow::anyhow!("subagent emitted no CONSOLIDATED/CONSOLIDATE_FAILED line")
        })?;

    if let Some(reason) = line.strip_prefix("CONSOLIDATE_FAILED") {
        anyhow::bail!("consolidate reported failure:{}", reason);
    }
    Ok((field(line, "promoted"), field(line, "deleted")))
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

    #[test]
    fn parse_consolidated_reads_counts() {
        let (p, d) = parse_consolidated("noise\nCONSOLIDATED promoted=2 deleted=4").unwrap();
        assert_eq!((p, d), (2, 4));
    }

    #[test]
    fn parse_consolidated_zeroed_ok() {
        let (p, d) = parse_consolidated("CONSOLIDATED promoted=0 deleted=0").unwrap();
        assert_eq!((p, d), (0, 0));
    }

    #[test]
    fn parse_consolidated_failure_and_missing_are_err() {
        assert!(parse_consolidated("CONSOLIDATE_FAILED store unreadable").is_err());
        assert!(parse_consolidated("I did some stuff but forgot the line").is_err());
    }


    #[test]
    fn parse_episodic_rows_handles_array_and_ndjson() {
        let now = Utc::now().to_rfc3339();
        let array = format!(
            r#"[{{"id":"a","content":"x","type":"fact","contexts":[],"created_at":"{now}"}}]"#
        );
        assert_eq!(parse_episodic_rows(&array).unwrap().len(), 1);

        let ndjson = format!(
            "{{\"id\":\"a\",\"content\":\"x\",\"type\":\"fact\",\"created_at\":\"{now}\"}}\n\
             {{\"id\":\"b\",\"content\":\"y\",\"type\":\"learned\",\"created_at\":\"{now}\"}}"
        );
        assert_eq!(parse_episodic_rows(&ndjson).unwrap().len(), 2);

        assert_eq!(parse_episodic_rows("  ").unwrap().len(), 0);
    }

    #[test]
    fn decay_ts_prefers_updated_at() {
        let created = Utc::now() - ChronoDuration::days(10);
        let updated = Utc::now() - ChronoDuration::days(1);
        let row = EpisodicRow {
            id: "x".into(),
            content: "c".into(),
            fact_type: "fact".into(),
            contexts: vec![],
            created_at: created.to_rfc3339(),
            updated_at: Some(updated.to_rfc3339()),
        };
        // Round-trips through RFC-3339, so compare at second precision.
        assert_eq!(row.decay_ts().timestamp(), updated.timestamp());

        let row2 = EpisodicRow {
            updated_at: None,
            ..row
        };
        assert_eq!(row2.decay_ts().timestamp(), created.timestamp());
    }

    #[test]
    fn decay_ts_unparseable_is_epoch() {
        let row = EpisodicRow {
            id: "x".into(),
            content: "c".into(),
            fact_type: "fact".into(),
            contexts: vec![],
            created_at: "not-a-timestamp".into(),
            updated_at: None,
        };
        assert_eq!(row.decay_ts(), DateTime::<Utc>::UNIX_EPOCH);
    }
}

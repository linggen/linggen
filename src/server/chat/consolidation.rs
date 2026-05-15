//! Every-N-turns memory consolidation trigger + tick.
//!
//! Fired from the post-turn seam in [`super::handler`] once a session has
//! completed a multiple of `consolidate_every_n_turns` turns. The cadence
//! is *derived* from chat history (see
//! [`crate::engine::memory::should_consolidate`]) — there is no persisted
//! counter, so it is per-session and restart-safe by construction.
//!
//! Pipeline per tick (locked contract, `memory-spec.md` §2):
//!
//! 1. **Encode** + 2. **Consolidate** — one non-interactive `ling-mem`
//!    subagent (`agents/ling-mem.md`). Its only tool is Bash (spec
//!    `tools: ["Bash"]`); the task prompt directs it to the resolved
//!    binary path. There is no `bash_allow_prefixes` lock — `tools:
//!    ["Bash"]` + the tight non-interactive prompt is the boundary,
//!    accepted for an internal agent whose only inputs are the user's
//!    own transcript and engine-selected rows. Encodes the recent
//!    exchange into the episodic table, then terminally
//!    promotes/deletes the *past-TTL* worklist the engine pre-selected.
//! 3. **Evict** — deterministic engine code (no LLM): `ling-mem evict`
//!    backstops any past-TTL row the subagent failed to reach.
//!
//! The engine owns TTL policy: it computes the absolute cutoff once,
//! selects the past-TTL worklist itself (binary stays policy-free), and
//! hands it to the subagent. See `project_consolidator_contract` memory.

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

/// Parsed result of one tick — drives the task #6 widget (material change
/// = any of promoted/superseded/deleted > 0). For now it is only logged.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct TickOutcome {
    pub encoded: u32,
    pub promoted: u32,
    pub superseded: u32,
    pub deleted: u32,
}

impl TickOutcome {
    /// A persistent result line lands only on a material change
    /// (`memory-spec.md` §2) — ≥1 row promoted or superseded.
    fn is_material(&self) -> bool {
        self.promoted > 0 || self.superseded > 0
    }
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
    let episodic_ttl_days = engine.cfg.episodic_ttl_days;

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
            episodic_ttl_days,
            recent_transcript,
        )
        .await;

        match result {
            Ok(outcome) if outcome.is_material() => tracing::info!(
                session_id = %session_id,
                ?outcome,
                "memory consolidation tick: material change"
            ),
            Ok(outcome) => tracing::debug!(
                session_id = %session_id,
                ?outcome,
                "memory consolidation tick: no-op"
            ),
            // Surfaced as an explicit failure widget in task #6; for now
            // the trigger wiring just records it.
            Err(e) => tracing::warn!(
                session_id = %session_id,
                error = %e,
                "memory consolidation tick failed"
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

/// One encode → consolidate → evict pass for `session_id`.
async fn run_consolidation_tick(
    state: &Arc<ServerState>,
    manager: &Arc<AgentManager>,
    session_id: &str,
    agent_id: &str,
    ws_root: &PathBuf,
    episodic_ttl_days: u64,
    recent_transcript: String,
) -> anyhow::Result<TickOutcome> {
    // The engine owns TTL policy: one absolute cutoff drives both the
    // worklist selection and the evict backstop.
    let cutoff = Utc::now() - ChronoDuration::days(episodic_ttl_days as i64);

    // `ling-mem` lives at `$SKILL_DIR/bin/ling-mem` (memory skill install)
    // with a `$PATH` fallback. Resolve once; the task prompt directs the
    // subagent to this absolute path so commands resolve regardless of
    // PATH (this is for resolvability, not a security lock — see the
    // module doc on the boundary).
    let skill_dir = state
        .skill_manager
        .active_provider("memory")
        .await
        .and_then(|s| s.skill_dir);
    let bin = crate::engine::capability_tools::resolve_binary(skill_dir.as_deref(), "ling-mem");
    let data_dir = crate::paths::linggen_home();

    // Step pre-work (deterministic): list episodic, filter to past-TTL.
    // `--data-dir` (highest precedence) avoids any env/cwd ambiguity.
    let worklist = match select_past_ttl_worklist(&bin, &data_dir, cutoff).await {
        Ok(rows) => rows,
        Err(e) => {
            // Episodic unreadable (e.g. the known pre-Phase-1b tier
            // migration gap) — don't fabricate a tick. Surface and bail.
            return Err(e.context("listing episodic rows for consolidation"));
        }
    };

    if recent_transcript.trim().is_empty() && worklist.is_empty() {
        return Ok(TickOutcome::default());
    }

    // Steps 1+2: hand the subagent the exact binary, the cutoff, the
    // exchange to encode, and the pre-selected worklist to terminally
    // decide. Judgment is the agent's; mechanics are the binary's.
    let task =
        build_task_prompt(session_id, &bin, &data_dir, &cutoff, &recent_transcript, &worklist);

    // Run via the standard delegation path — same lifecycle every
    // subagent uses: run-record tracking, cancellation, and the
    // SubagentSpawned/SubagentResult/AgentStatus events the widget (#6)
    // consumes. `parent_interactive=false` keeps it unattended (no
    // prompts → no deadlock); the `ling-mem` agent spec already pins
    // `tools: ["Bash"]`, and the task carries the absolute binary path so
    // commands resolve without relying on PATH.
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
        None,     // parent_policy — spec tools restrict it
        Vec::new(), // parent_path_modes
        false,    // parent_interactive
    )
    .await
    .map_err(|e| e.context("ling-mem consolidation subagent"))?;

    use crate::engine::tools::ToolResult;
    let summary = match result {
        ToolResult::Success(text) => text,
        // The Bash-only non-interactive agent should always finish with
        // AgentOutcome::None → Success. Anything else (an unexpected
        // plan/done outcome) is a bug worth seeing in the log.
        ToolResult::AgentOutcome(o) => {
            anyhow::bail!("consolidation subagent ended with outcome {o:?}, not a result")
        }
        other => anyhow::bail!("consolidation subagent returned {other:?}"),
    };

    // Step 3 (deterministic backstop): evict whatever past-TTL rows the
    // subagent didn't terminally handle. Same cutoff. Failures here are
    // non-fatal — the rows simply age out on a later tick.
    if let Err(e) = run_ling_mem(
        &bin,
        &data_dir,
        &["evict", "--before", &cutoff.to_rfc3339()],
    )
    .await
    {
        tracing::warn!(session_id = %session_id, error = %e, "evict backstop failed (non-fatal)");
    }

    parse_outcome(&summary)
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

/// The subagent task. Carries the literal binary path (the agent is
/// instructed to invoke exactly this — for PATH-independent
/// resolvability, not a hard lock), the data dir, the exchange to
/// encode, and the pre-selected past-TTL worklist.
fn build_task_prompt(
    session_id: &str,
    bin: &Path,
    data_dir: &Path,
    cutoff: &DateTime<Utc>,
    recent_transcript: &str,
    worklist: &[EpisodicRow],
) -> String {
    let bin = bin.display();
    let dd = data_dir.display();
    let today = Utc::now().format("%Y-%m-%d");

    let worklist_block = if worklist.is_empty() {
        "(none — the worklist is empty; do Step 1 only, then emit the line with promoted=0 superseded=0 deleted=0)".to_string()
    } else {
        worklist
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
            .join("\n")
    };

    format!(
        "Memory consolidation tick for session `{session}`. Today is {today}.\n\
         \n\
         Invoke the binary as EXACTLY this path on every command (commands \
         must start with it, then `--data-dir {dd}`):\n\
         `{bin}`\n\
         Example: `{bin} --data-dir {dd} add \"...\" --episodic --type fact --from user`\n\
         \n\
         ## Step 1 — encode this recent exchange into episodic\n\
         Apply the exclusion filters from your instructions. Date-stamp \
         ages relative to {today}.\n\
         <recent-exchange>\n{transcript}\n</recent-exchange>\n\
         \n\
         ## Step 2 — terminally decide each past-TTL worklist row\n\
         These episodic rows are older than the {cutoff} TTL cutoff. The \
         engine already selected them — do NOT list episodic yourself. \
         For EACH: promote (write to the semantic store, then \
         `{bin} --data-dir {dd} delete <id> --episodic --yes`) or delete \
         (`{bin} --data-dir {dd} delete <id> --episodic --yes`).\n\
         <worklist>\n{worklist_block}\n</worklist>\n\
         \n\
         Then emit your single status line and stop.",
        session = session_id,
        today = today,
        bin = bin,
        dd = dd,
        transcript = recent_transcript,
        cutoff = cutoff.to_rfc3339(),
        worklist_block = worklist_block,
    )
}

/// Parse the subagent's contract line:
/// `CONSOLIDATED encoded=<n> promoted=<n> superseded=<n> deleted=<n>` or
/// `CONSOLIDATE_FAILED <reason>`.
fn parse_outcome(summary: &str) -> anyhow::Result<TickOutcome> {
    let line = summary
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with("CONSOLIDATED") || l.starts_with("CONSOLIDATE_FAILED"))
        .ok_or_else(|| {
            anyhow::anyhow!("subagent emitted no CONSOLIDATED/CONSOLIDATE_FAILED line")
        })?;

    if let Some(reason) = line.strip_prefix("CONSOLIDATE_FAILED") {
        anyhow::bail!("subagent reported failure:{}", reason);
    }

    let mut out = TickOutcome::default();
    for tok in line.split_whitespace() {
        let Some((k, v)) = tok.split_once('=') else {
            continue;
        };
        let n: u32 = v.parse().unwrap_or(0);
        match k {
            "encoded" => out.encoded = n,
            "promoted" => out.promoted = n,
            "superseded" => out.superseded = n,
            "deleted" => out.deleted = n,
            _ => {}
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_outcome_reads_counts() {
        let out =
            parse_outcome("some chatter\nCONSOLIDATED encoded=3 promoted=2 superseded=1 deleted=4")
                .unwrap();
        assert_eq!((out.encoded, out.promoted, out.superseded, out.deleted), (3, 2, 1, 4));
        assert!(out.is_material());
    }

    #[test]
    fn parse_outcome_zeroed_is_not_material() {
        let out = parse_outcome("CONSOLIDATED encoded=0 promoted=0 superseded=0 deleted=0").unwrap();
        assert!(!out.is_material());
    }

    #[test]
    fn parse_outcome_failure_is_err() {
        let err = parse_outcome("CONSOLIDATE_FAILED episodic store unreadable").unwrap_err();
        assert!(err.to_string().contains("subagent reported failure"));
    }

    #[test]
    fn parse_outcome_missing_line_is_err() {
        assert!(parse_outcome("I did some stuff but forgot the line").is_err());
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

//! Built-in core memory — `tier=core` rows pulled from the user's
//! memory store and injected into every owner session.
//!
//! Per `doc/memory-spec.md` §1/§2 the core tier lives as rows in the
//! `semantic` LanceDB table, not as files on disk. The engine queries
//! them via the `ling-mem` CLI (which transparently routes through the
//! HTTP daemon when one is running) and renders the bodies as a bullet
//! list for the prompt template. Promotion in and out of the core tier
//! happens through ordinary memory writes
//! (`Memory_write({verb: "add", tier: "core", ...})`) and the dashboard
//! — there is no second markdown substrate to keep in sync.

use serde::Deserialize;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Binary name. Resolved against `$PATH`; the installer puts `ling-mem`
/// there. Missing binary degrades gracefully: `load_core` returns `None`
/// and the prompt falls through to the empty-core block.
const LING_MEM_BIN: &str = "ling-mem";

/// Upper bound on rows pulled into the system prompt. Core is meant to
/// be tiny (a handful of universals); the cap keeps a runaway tag from
/// silently inflating every prompt.
const CORE_LIMIT: usize = 200;

/// Wall-clock cap on the `ling-mem list --tier core` subprocess.
/// `load_core` runs on every stable-prompt build (per session start /
/// rebuild), so a hung daemon — e.g. ling-mem holding a LanceDB lock
/// during an encoder run — would freeze the whole engine loop before
/// the model request ever fires. The cap means "no core injected this
/// turn" instead of "agent stuck indefinitely". 2s is generous for a
/// localhost JSON list call yet still bounds the worst case.
const LOAD_CORE_TIMEOUT: Duration = Duration::from_secs(2);

/// Pre-rendered core memory block. `facts` is the markdown body the
/// prompt template inlines verbatim — a bullet list of `tier=core` row
/// contents, newest first.
pub(crate) struct CoreContent {
    pub facts: String,
}

#[derive(Deserialize)]
struct CoreRow {
    content: String,
}

/// Query `tier=core` rows from `ling-mem` and render them as a bullet
/// list. Returns `None` when there are no core rows (or the binary is
/// unavailable / errors out — the caller emits the empty-block prompt
/// in that case so a fresh install still starts cleanly).
pub(crate) fn load_core() -> Option<CoreContent> {
    let stdout = run_with_timeout(LOAD_CORE_TIMEOUT)?;
    let rows = parse_ndjson_rows(&stdout)?;
    if rows.is_empty() {
        return None;
    }

    let facts = rows
        .iter()
        .map(|r| format!("- {}", r.content.trim()))
        .collect::<Vec<_>>()
        .join("\n");
    Some(CoreContent { facts })
}

/// Spawn `ling-mem list --tier core` and bound the wait by `timeout`.
/// Returns stdout on success; `None` on spawn error, non-zero exit,
/// timeout, or stdout that isn't valid UTF-8. On timeout the child is
/// killed so it doesn't linger zombie.
fn run_with_timeout(timeout: Duration) -> Option<String> {
    let mut child = Command::new(LING_MEM_BIN)
        .args([
            "--format",
            "json",
            "list",
            "--tier",
            "core",
            "--limit",
        ])
        .arg(CORE_LIMIT.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    // Wait for the child on a side thread so we can race it against a
    // wall-clock timeout. `child.wait_timeout` would be cleaner but
    // needs the `wait-timeout` crate; this approach uses only std.
    let (tx, rx) = mpsc::channel();
    let stdout = child.stdout.take();
    let pid = child.id();
    thread::spawn(move || {
        let result = (|| -> Option<(std::process::ExitStatus, String)> {
            let mut buf = String::new();
            if let Some(mut out) = stdout {
                let _ = out.read_to_string(&mut buf);
            }
            let status = child.wait().ok()?;
            Some((status, buf))
        })();
        let _ = tx.send(result);
    });

    match rx.recv_timeout(timeout) {
        Ok(Some((status, buf))) => {
            if !status.success() {
                tracing::debug!(
                    status = ?status.code(),
                    "ling-mem list --tier core exited non-zero; treating core as empty"
                );
                return None;
            }
            Some(buf)
        }
        Ok(None) => None,
        Err(_) => {
            // Subprocess overran the budget. The engine loop proceeds
            // with no core injected this turn; the orphan finishes
            // whenever the daemon unblocks. (Cleaner kill-on-timeout
            // would require sharing the child handle across threads
            // or pulling in `wait-timeout` — not worth the complexity
            // for a 2s budget; an idle ling-mem invocation is cheap.)
            tracing::warn!(
                pid,
                ?timeout,
                "ling-mem list --tier core timed out; treating core as empty"
            );
            None
        }
    }
}

/// Parse `ling-mem`'s NDJSON list output (one JSON object per line).
/// Tolerates trailing whitespace and blank lines. Returns `None` only if
/// every non-blank line failed to parse — a partial parse keeps the rows
/// it did get so a single malformed row can't blank out the entire core.
fn parse_ndjson_rows(stdout: &str) -> Option<Vec<CoreRow>> {
    let mut rows = Vec::new();
    let mut had_any_line = false;
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        had_any_line = true;
        match serde_json::from_str::<CoreRow>(trimmed) {
            Ok(row) => rows.push(row),
            Err(e) => {
                tracing::debug!(error = %e, "skipping malformed ling-mem row");
            }
        }
    }
    if !had_any_line {
        return Some(Vec::new());
    }
    Some(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ndjson_list_output() {
        let stdout = "{\"content\":\"I'm a founder\"}\n{\"content\":\"Prefer terse replies\"}\n";
        let rows = parse_ndjson_rows(stdout).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].content, "I'm a founder");
        assert_eq!(rows[1].content, "Prefer terse replies");
    }

    #[test]
    fn empty_stdout_returns_empty_rows() {
        let rows = parse_ndjson_rows("").unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn malformed_lines_are_skipped_not_fatal() {
        let stdout = "{\"content\":\"keep me\"}\nnot-json\n{\"content\":\"and me\"}\n";
        let rows = parse_ndjson_rows(stdout).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].content, "keep me");
        assert_eq!(rows[1].content, "and me");
    }
}

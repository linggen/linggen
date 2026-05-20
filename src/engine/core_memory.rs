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
use std::process::Command;

/// Binary name. Resolved against `$PATH`; the installer puts `ling-mem`
/// there. Missing binary degrades gracefully: `load_core` returns `None`
/// and the prompt falls through to the empty-core block.
const LING_MEM_BIN: &str = "ling-mem";

/// Upper bound on rows pulled into the system prompt. Core is meant to
/// be tiny (a handful of universals); the cap keeps a runaway tag from
/// silently inflating every prompt.
const CORE_LIMIT: usize = 200;

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
    let output = Command::new(LING_MEM_BIN)
        .args([
            "--format",
            "json",
            "list",
            "--tier",
            "core",
            "--limit",
        ])
        .arg(CORE_LIMIT.to_string())
        .output()
        .ok()?;

    if !output.status.success() {
        tracing::debug!(
            status = ?output.status.code(),
            "ling-mem list --tier core exited non-zero; treating core as empty"
        );
        return None;
    }

    let stdout = std::str::from_utf8(&output.stdout).ok()?;
    let rows = parse_ndjson_rows(stdout)?;
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

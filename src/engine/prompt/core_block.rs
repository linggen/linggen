//! Built-in core memory — `tier=core` rows pulled from the user's
//! memory store and injected into every owner session.
//!
//! Per `doc/memory-spec.md` §1/§2 the core tier lives as rows in the
//! `semantic` LanceDB table, not as files on disk. The engine queries
//! them over HTTP from the daemon at `agent.ling_mem_url` — the same
//! wire path recall and `Memory_*` dispatch use, so every memory
//! surface reads the same store — and renders the bodies as a bullet
//! list for the prompt template. Promotion in and out of the core tier
//! happens through ordinary memory writes
//! (`Memory_write({verb: "add", tier: "core", ...})`) and the dashboard
//! — there is no second markdown substrate to keep in sync.

use serde::Deserialize;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Upper bound on rows pulled into the system prompt. Core is meant to
/// be tiny (a handful of universals); the cap keeps a runaway tag from
/// silently inflating every prompt.
const CORE_LIMIT: usize = 200;

/// Wall-clock cap on the core list call. `load_core` runs on every
/// stable-prompt build (per session start / rebuild), so a hung daemon
/// — e.g. ling-mem holding a LanceDB lock during an encoder run —
/// would freeze the whole engine loop before the model request ever
/// fires. The cap means "no core injected this turn" instead of "agent
/// stuck indefinitely". 2s is generous for a localhost JSON list call
/// yet still bounds the worst case.
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
    /// Row id — surfaced in the rendered line so the agent can act on
    /// duplicates / conflicts directly (`ling-mem delete <id>`,
    /// `Memory_write({verb:"add", replace_ids:[<id>], ...})`). Without
    /// this, the agent sees content-only bullets and has to round-trip
    /// through `Memory_query` just to learn the ids. Optional so a
    /// malformed row degrades to a content-only line instead of failing
    /// to parse the whole batch.
    #[serde(default)]
    id: Option<String>,
    #[serde(default, rename = "type")]
    row_type: Option<String>,
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
}

/// Footer instruction appended after the rows when there are ≥2 of them.
/// Same intent as CC's `recall.sh` footer (so linggen and CC speak one
/// reconcile-on-recall protocol), but (a) rephrased to use the
/// linggen-native `Memory_*` tools instead of the `ling-mem` CLI — in
/// owner sessions the agent dispatches through the memory capability,
/// never shelling out — and (b) carrying the "don't ambush" gate inline
/// so the rule and its scope live in one place instead of forcing the
/// model to reconcile a footer instruction against a separate user
/// preference row. Single-row blocks skip the footer — nothing to dedup
/// or compare against.
pub(crate) const RECONCILE_FOOTER: &str = "\n\nNote: If duplicates or conflicting rows appear above AND the user's current turn is unrelated to memory itself (incidental recall hit), resolve them on the side — merge authority follows voice: `Memory_write({verb:\"delete\", id:\"<id>\"})` for exact dups; rows that are all your own notes (from=derived — built/fixed/tried/learned) merge freely into one current-truth row via `Memory_write({verb:\"add\", content:\"<merged>\", replace_ids:[\"<loser_id_1>\", ...], ...})`, no AskUser; if any row is in the user's voice (from=user — preference/decision/identity), AskUser first, then the same atomic `replace_ids` write (never separate add + delete). If the user IS explicitly steering memory (\"clean up\", \"remember X\", \"what's in memory\", \"ignore the hits\"), follow their instruction and do NOT side-quest into dedup. Either way, keep memory in good shape.";

/// Per-turn capture nudge, injected model-only every owner turn (NOT
/// rendered in the recall widget — it's an instruction to the agent, not a
/// recalled memory). Word-for-word aligned with CC/Codex `recall.sh` so the
/// per-turn reminder reads identically across every host. Definitions /
/// routing live in `[memory_protocol]` (session start); this only nudges.
pub(crate) const CAPTURE_REMINDER: &str = "Memory capture: before finishing this turn, recognize anything worth remembering and write it at the right tier per the memory protocol (core/semantic = search-first; episodic = incidental); anchor relative time to absolute dates (\"last month\" → \"2026-06\"). Nothing worth keeping? Skip silently.";

/// Query `tier=core` rows from the daemon at `ling_mem_url` and render
/// them as a bullet list. Returns `None` when there are no core rows
/// (or the daemon is unreachable / errors out — the caller emits the
/// empty-block prompt in that case so a fresh install still starts
/// cleanly).
pub(crate) fn load_core(ling_mem_url: &str) -> Option<CoreContent> {
    let rows = fetch_core_rows(ling_mem_url, LOAD_CORE_TIMEOUT)?;
    if rows.is_empty() {
        return None;
    }

    let bullets = rows
        .iter()
        .map(render_row)
        .collect::<Vec<_>>()
        .join("\n");
    let facts = if rows.len() > 1 {
        format!("{bullets}{RECONCILE_FOOTER}")
    } else {
        bullets
    };
    Some(CoreContent { facts })
}

/// `- (type, host, YYYY-MM-DD, id=<id>): content` — matches the line
/// shape `recall.sh` prints, so a row reads the same in linggen's core
/// block and in CC's recall context.
fn render_row(r: &CoreRow) -> String {
    let row_type = r.row_type.as_deref().unwrap_or("fact");
    let host = r.host.as_deref().unwrap_or("unknown");
    let date = r
        .created_at
        .as_deref()
        .map(|s| &s[..s.len().min(10)])
        .unwrap_or("");
    let content = r.content.trim();
    match r.id.as_deref() {
        Some(id) => format!("- ({row_type}, {host}, {date}, id={id}): {content}"),
        None => format!("- ({row_type}, {host}, {date}): {content}"),
    }
}

/// List `tier=core` rows over HTTP, bounded by `timeout`. Goes through
/// `call_memory_http` — the exact wire path recall and `Memory_*`
/// dispatch use — so the core block honors `agent.ling_mem_url` like
/// every other memory surface (a non-default URL, e.g. an eval's
/// throwaway store, reads the right daemon). The request runs on a side
/// thread with its own tiny runtime because prompt assembly is sync; on
/// timeout the engine proceeds with no core injected this turn and the
/// orphan request finishes in the background (including the dispatch
/// layer's one autostart retry, which helps the next turn).
fn fetch_core_rows(ling_mem_url: &str, timeout: Duration) -> Option<Vec<CoreRow>> {
    let (tx, rx) = mpsc::channel();
    let url = ling_mem_url.to_string();
    thread::spawn(move || {
        let result = (|| -> Option<Vec<CoreRow>> {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()?;
            let value = rt
                .block_on(crate::engine::tools::memory_tool::call_memory_http(
                    &url,
                    "Memory_query",
                    serde_json::json!({"verb": "list", "tier": "core", "limit": CORE_LIMIT}),
                ))
                .map_err(|e| tracing::debug!(error = %e, "core list over ling-mem HTTP failed; treating core as empty"))
                .ok()?;
            parse_rows(&value)
        })();
        let _ = tx.send(result);
    });

    match rx.recv_timeout(timeout) {
        Ok(rows) => rows,
        Err(_) => {
            tracing::warn!(
                ?timeout,
                "core list over ling-mem HTTP timed out; treating core as empty"
            );
            None
        }
    }
}

/// Parse the daemon's list payload (a JSON array of rows). A malformed
/// row degrades to a skipped line instead of blanking the whole core.
fn parse_rows(value: &serde_json::Value) -> Option<Vec<CoreRow>> {
    let arr = value.as_array()?;
    let rows = arr
        .iter()
        .filter_map(|v| match serde_json::from_value::<CoreRow>(v.clone()) {
            Ok(row) => Some(row),
            Err(e) => {
                tracing::debug!(error = %e, "skipping malformed ling-mem row");
                None
            }
        })
        .collect();
    Some(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_list_payload() {
        let value = serde_json::json!([
            {"content": "I'm a founder"},
            {"content": "Prefer terse replies"},
        ]);
        let rows = parse_rows(&value).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].content, "I'm a founder");
        assert_eq!(rows[1].content, "Prefer terse replies");
    }

    #[test]
    fn empty_payload_returns_empty_rows() {
        let rows = parse_rows(&serde_json::json!([])).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn malformed_rows_are_skipped_not_fatal() {
        let value = serde_json::json!([
            {"content": "keep me"},
            {"no_content_field": true},
            {"content": "and me"},
        ]);
        let rows = parse_rows(&value).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].content, "keep me");
        assert_eq!(rows[1].content, "and me");
    }

    #[test]
    fn non_array_payload_returns_none() {
        assert!(parse_rows(&serde_json::json!({"ok": true})).is_none());
    }
}

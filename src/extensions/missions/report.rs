//! Engine-composed mission run report.
//!
//! When a mission run completes, the scheduler appends one summary
//! message to the run's session, composed mechanically from the
//! transcript's memory tool results (the `days` worklist,
//! `remember_day` stamps, `sweep` evictions). The model's own status
//! lines stay best-effort narration; this report is the trustworthy
//! record of what actually happened — a model was observed inventing
//! failure reasons and a wrong final token instead of reporting
//! (dream run, 2026-07-06).

use crate::state_fs::sessions::ChatMsg;

const WRITE_OK: &str = "Tool Memory_write: success: ";
const QUERY_OK: &str = "Tool Memory_query: success: ";

/// Compose a run report from the session's memory tool results.
/// `None` when the run stamped no day and ran no sweep — missions that
/// don't drive the memory pipeline stay silent.
pub(crate) fn compose_memory_report(messages: &[ChatMsg]) -> Option<String> {
    let mut worklist: Option<String> = None;
    let mut remembered: Vec<String> = Vec::new();
    let mut swept: Vec<String> = Vec::new();
    let mut merges = 0u64;
    let mut retired = 0u64;

    for m in messages.iter().filter(|m| m.from_id == "system") {
        if let Some(json) = m.content.strip_prefix(WRITE_OK) {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
                continue;
            };
            if let Some(line) = remember_line(&v) {
                remembered.push(line);
            } else if let Some(line) = sweep_line(&v) {
                swept.push(line);
            } else if let Some(n) = replaced_count(&v) {
                merges += 1;
                retired += n;
            }
        } else if worklist.is_none() {
            if let Some(json) = m.content.strip_prefix(QUERY_OK) {
                worklist = worklist_line(json);
            }
        }
    }

    if remembered.is_empty() && swept.is_empty() && merges == 0 {
        return None;
    }
    let condensed = (merges > 0).then(|| {
        format!("condensed: {merges} chain(s) merged, {retired} superseded rows retired")
    });
    let lines: Vec<String> = worklist
        .into_iter()
        .chain(remembered)
        .chain(swept)
        .chain(condensed)
        .collect();
    Some(format!("Mission report:\n- {}", lines.join("\n- ")))
}

/// An add that carried `replace_ids` — the daemon reports the retired
/// losers in `replaced`. Counts both condense chain-collapses and
/// see-it-solve-it merges during a remember pass.
fn replaced_count(v: &serde_json::Value) -> Option<u64> {
    let replaced = v.get("replaced")?.as_array()?;
    if replaced.is_empty() {
        return None;
    }
    Some(replaced.len() as u64)
}

/// The run's first `days` rollup — how many days awaited a dream at
/// start. Search/list results are JSON arrays and `get` returns a bare
/// fact, so only the days-rollup object (a `days` array) matches here.
/// The rollup may be the FULL calendar (a day-scoped run's context
/// fetch), so count only undreamed entries (past day, unjudged rows) —
/// never the raw array length.
fn worklist_line(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let days = v.get("days")?.as_array()?;
    let today = v.get("today").and_then(|t| t.as_str()).unwrap_or("");
    let undreamed: Vec<&serde_json::Value> = days
        .iter()
        .filter(|d| {
            let date = d.get("date").and_then(|s| s.as_str()).unwrap_or("");
            let unjudged = d.get("unjudged").and_then(|n| n.as_u64()).unwrap_or(0);
            unjudged > 0 && (today.is_empty() || date < today)
        })
        .collect();
    let Some(oldest) = undreamed.first() else {
        return Some("worklist: no undreamed days".to_string());
    };
    let date = oldest.get("date")?.as_str()?;
    let rows = oldest.get("rows").and_then(|r| r.as_u64()).unwrap_or(0);
    Some(match undreamed.len() {
        1 => format!("worklist: 1 undreamed day — {date} ({rows} rows)"),
        n => format!("worklist: {n} undreamed days, oldest {date} ({rows} rows)"),
    })
}

/// A `remember_day` stamp: `{"date": "...", "record": {"judged": n, "promoted": n, ...}}`.
fn remember_line(v: &serde_json::Value) -> Option<String> {
    let date = v.get("date")?.as_str()?;
    let record = v.get("record")?;
    let judged = record.get("judged").and_then(|n| n.as_u64()).unwrap_or(0);
    let promoted = record.get("promoted").and_then(|n| n.as_u64()).unwrap_or(0);
    Some(format!(
        "remembered {date}: {judged} judged, {promoted} promoted to long-term"
    ))
}

/// One store-state line from the daemon's `stats` response — appended
/// to the run report so every dream run ends with where the store
/// stands: `store now: 312 rows (core 9 · long-term 184 · short-term
/// 119) · 71.2 MB on disk`.
pub(crate) fn store_state_line(stats: &serde_json::Value) -> Option<String> {
    let total = stats.get("total")?.as_u64()?;
    let tier = stats.get("per_tier")?;
    let n = |k: &str| tier.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
    let disk = stats
        .get("disk_bytes")
        .and_then(|d| d.get("total"))
        .and_then(|v| v.as_u64())
        .map(|b| format!(" · {:.1} MB on disk", b as f64 / 1e6))
        .unwrap_or_default();
    Some(format!(
        "store now: {} rows (core {} · long-term {} · short-term {}){}",
        total,
        n("core"),
        n("semantic"),
        n("episodic"),
        disk
    ))
}

/// Review-queue line from the daemon's `stats` response — only when
/// items await the user, so quiet stores stay quiet:
/// `needs review: 3 item(s) — solve with /linggen:solve or the memory app`.
pub(crate) fn review_line(stats: &serde_json::Value) -> Option<String> {
    let open = stats.get("open_issues")?.as_u64()?;
    (open > 0).then(|| {
        format!("needs review: {open} item(s) — solve with /linggen:solve or the memory app")
    })
}

/// A `sweep` result: `{"days": {"<date>": n, ...}, "dry_run": bool, "removed": n}`.
fn sweep_line(v: &serde_json::Value) -> Option<String> {
    let removed = v.get("removed")?.as_u64()?;
    if v.get("dry_run").and_then(|d| d.as_bool()).unwrap_or(false) {
        return None;
    }
    if removed == 0 {
        return Some("forget sweep: nothing expired".to_string());
    }
    let by_day = v
        .get("days")
        .and_then(|d| d.as_object())
        .map(|m| {
            m.iter()
                .map(|(date, n)| format!("{date}: {n}"))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|s| !s.is_empty())
        .map(|s| format!(" ({s})"))
        .unwrap_or_default();
    Some(format!("forget sweep: removed {removed} expired rows{by_day}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sys(content: &str) -> ChatMsg {
        ChatMsg {
            agent_id: "memory".into(),
            from_id: "system".into(),
            to_id: "memory".into(),
            content: content.into(),
            timestamp: 0,
            is_observation: true,
        }
    }

    #[test]
    fn full_run_report() {
        // Shapes copied verbatim from the 2026-07-06 live run transcript.
        let messages = vec![
            sys(r#"Tool Memory_query: success: {"days":[{"date":"2026-07-03","dreamed":false,"forgotten":0,"harvested_at":null,"judged":0,"past_ttl":0,"promoted":0,"remembered_at":null,"rows":14,"scanned":false,"unjudged":14}],"today":"2026-07-06","ttl_days":7}"#),
            sys(r#"Tool Memory_query: success: [{"content":"unrelated search result"}]"#),
            sys(r#"Tool Memory_write: success: {"action":"added","fact":{"content":"a promoted row","id":"abc"}}"#),
            sys(r#"Tool Memory_write: success: {"date":"2026-07-03","record":{"forgotten":0,"judged":14,"promoted":7,"remembered_at":"2026-07-06T17:06:50Z"}}"#),
            sys(r#"Tool Memory_write: success: {"days":{"2026-06-29":4},"dry_run":false,"removed":4}"#),
        ];
        let report = compose_memory_report(&messages).unwrap();
        assert_eq!(
            report,
            "Mission report:\n\
             - worklist: 1 undreamed day — 2026-07-03 (14 rows)\n\
             - remembered 2026-07-03: 14 judged, 7 promoted to long-term\n\
             - forget sweep: removed 4 expired rows (2026-06-29: 4)"
        );
    }

    #[test]
    fn condense_merges_are_counted() {
        let messages = vec![
            sys(r#"Tool Memory_write: success: {"days":{},"dry_run":false,"removed":0}"#),
            sys(r#"Tool Memory_write: success: {"action":"added","fact":{"id":"s1"},"replaced":["a","b"]}"#),
            sys(r#"Tool Memory_write: success: {"action":"added","fact":{"id":"s2"},"replaced":["c"]}"#),
            sys(r#"Tool Memory_write: success: {"action":"added","fact":{"id":"plain"}}"#),
        ];
        let report = compose_memory_report(&messages).unwrap();
        assert_eq!(
            report,
            "Mission report:\n\
             - forget sweep: nothing expired\n\
             - condensed: 2 chain(s) merged, 3 superseded rows retired"
        );
    }

    #[test]
    fn no_op_run_reports_quiet_sweep() {
        let messages = vec![
            sys(r#"Tool Memory_query: success: {"days":[],"today":"2026-07-06","ttl_days":7}"#),
            sys(r#"Tool Memory_write: success: {"days":{},"dry_run":false,"removed":0}"#),
        ];
        let report = compose_memory_report(&messages).unwrap();
        assert_eq!(
            report,
            "Mission report:\n- worklist: no undreamed days\n- forget sweep: nothing expired"
        );
    }

    #[test]
    fn non_memory_run_stays_silent() {
        let messages = vec![
            sys("Tool Bash: success: ok"),
            sys(r#"Tool Memory_write: success: {"action":"added","fact":{"id":"x"}}"#),
        ];
        assert!(compose_memory_report(&messages).is_none());
    }

    #[test]
    fn dry_run_sweep_is_ignored() {
        let messages = vec![sys(
            r#"Tool Memory_write: success: {"days":{"2026-06-29":4},"dry_run":true,"removed":4}"#,
        )];
        assert!(compose_memory_report(&messages).is_none());
    }
}

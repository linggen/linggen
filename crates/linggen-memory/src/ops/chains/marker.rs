//! Tier-2 scan: provisional-state marker candidates.

use serde_json::{json, Value};
use std::collections::HashSet;

use super::cited::citation_edges;
use super::{is_derived_note, public_row};
use crate::memory::store::cosine_similarity;
use crate::memory::Memory as Row;
use crate::memory::Tier;

/// Provisional-state markers, matched case-insensitively. Deliberately a
/// fixed list, not config: tier-2 hits are LLM-confirmed downstream, so
/// recall beats precision here — but every entry should still scream
/// "this row describes an unfinished state".
const MARKERS_CI: &[&str] = &[
    "uncommitted",
    "not committed",
    "not started",
    "unbuilt",
    "deferred",
    "in progress",
    "awaiting",
    "still pending",
    "build pending",
    "impl pending",
];

/// Markers matched case-sensitively — lowercase forms are too common in
/// ordinary prose ("open the file", "todo lists").
const MARKERS_CS: &[&str] = &["OPEN:", "OPEN(", "TODO:", "NEXT:", "PENDING"];

/// Neighbors below this cosine are noise, not chain candidates. Sits
/// under the restatement band (0.78–0.97 on the real Qwen3 distribution)
/// and above unrelated pairs (≤0.65) — chain members are related but not
/// restatements, so the floor leans permissive.
const NEIGHBOR_FLOOR: f32 = 0.60;
const NEIGHBORS_PER_ROW: usize = 5;

// ── Tier-2: marker candidates ───────────────────────────────────────────────

/// Rows carrying provisional-state markers, each with its nearest
/// stored-vector neighbors. Rows already inside a cited chain are
/// skipped — the tier-1 pass owns them. Rows named by a review issue
/// (`issued`, any status) are skipped too — queueing consumes nothing,
/// so without this the same oldest candidates re-queue as "already
/// queued" forever and the scan never reaches the rest of the store.
pub(super) fn marker_candidates(
    rows: &[Row],
    limit: usize,
    offset: usize,
    derived_only: bool,
    issued: &HashSet<String>,
) -> Value {
    let in_cited: Vec<bool> = {
        let edges = citation_edges(rows);
        let mut flags = vec![false; rows.len()];
        for (a, b) in edges {
            flags[a] = true;
            flags[b] = true;
        }
        flags
    };

    let (marked, queued_skipped): (Vec<(usize, &'static str)>, usize) = {
        let mut kept = Vec::new();
        let mut skipped = 0usize;
        let candidates = rows
            .iter()
            .enumerate()
            .filter(|(i, row)| !in_cited[*i] && row.tier != Tier::Core)
            .filter(|(_, row)| !derived_only || is_derived_note(row))
            .filter_map(|(i, row)| find_marker(&row.content).map(|m| (i, m)));
        for (i, marker) in candidates {
            if issued.contains(&rows[i].id) {
                skipped += 1;
            } else {
                kept.push((i, marker));
            }
        }
        (kept, skipped)
    };

    let total = marked.len();
    // Neighbors are computed for the requested page only — the O(N)
    // cosine sweep per row is cheap, but not worth doing 200× per call.
    let page: Vec<Value> = marked
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(i, marker)| {
            json!({
                "marker": marker,
                "derived_only": is_derived_note(&rows[i]),
                "row": public_row(&rows[i]),
                "neighbors": neighbors_of(rows, i),
            })
        })
        .collect();

    json!({
        "kind": "marker",
        "scanned": rows.len(),
        "total": total,
        "offset": offset,
        // Marker rows hidden because a review issue already names them —
        // reported so a caller never mistakes a filtered scan for a clean
        // store.
        "queued_skipped": queued_skipped,
        "candidates": page,
    })
}

fn find_marker(content: &str) -> Option<&'static str> {
    if let Some(m) = MARKERS_CS.iter().find(|m| content.contains(*m)) {
        return Some(m);
    }
    let lower = content.to_lowercase();
    MARKERS_CI.iter().find(|m| lower.contains(*m)).copied()
}

/// Top stored-vector neighbors of `rows[idx]` above the floor.
fn neighbors_of(rows: &[Row], idx: usize) -> Vec<Value> {
    let Some(query) = rows[idx].vector.as_deref() else {
        return Vec::new();
    };
    let mut scored: Vec<(f32, usize)> = rows
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != idx)
        .filter_map(|(i, row)| {
            let v = row.vector.as_deref()?;
            let score = cosine_similarity(query, v);
            (score >= NEIGHBOR_FLOOR).then_some((score, i))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored
        .into_iter()
        .take(NEIGHBORS_PER_ROW)
        .map(|(score, i)| {
            json!({
                "score": (score * 1000.0).round() / 1000.0,
                "row": public_row(&rows[i]),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::Origin;
    use crate::ops::chains::row;

    #[test]
    fn marker_rows_flagged_without_cited_members() {
        let rows = vec![
            row(
                "aaaaaaaaaa",
                "condense build OPEN: chains verb pending",
                Origin::Derived,
            ),
            row("bbbbbbbbbb", "merged, see aaaaaaaaaa", Origin::Derived),
            row(
                "cccccccccc",
                "feature X design locked, impl not started",
                Origin::Derived,
            ),
        ];
        let out = marker_candidates(&rows, 10, 0, false, &HashSet::new());
        // aaaaaaaaaa is in a cited chain → excluded; cccccccccc matches.
        assert_eq!(out["total"], 1);
        assert_eq!(out["candidates"][0]["row"]["id"], "cccccccccc");
        assert_eq!(out["candidates"][0]["marker"], "not started");
        assert_eq!(out["queued_skipped"], 0);
    }

    #[test]
    fn marker_rows_named_by_issues_are_skipped_and_counted() {
        let rows = vec![
            row(
                "aaaaaaaaaa",
                "migration deferred until the schema lands",
                Origin::Derived,
            ),
            row(
                "bbbbbbbbbb",
                "feature X design locked, impl not started",
                Origin::Derived,
            ),
        ];
        let issued: HashSet<String> = ["aaaaaaaaaa".to_string()].into();
        let out = marker_candidates(&rows, 10, 0, false, &issued);
        // The queued row no longer burns a slot; the fresh one surfaces.
        assert_eq!(out["total"], 1);
        assert_eq!(out["queued_skipped"], 1);
        assert_eq!(out["candidates"][0]["row"]["id"], "bbbbbbbbbb");
    }
}

//! Tier-1 scan: citation chains.

use serde_json::{json, Value};
use std::collections::HashMap;

use super::{is_derived_note, public_row};
use crate::memory::Memory as Row;

// ── Tier-1: citation chains ─────────────────────────────────────────────────

/// Verbatim id-citation edges → connected components of size ≥ 2.
pub(super) fn cited_chains(rows: &[Row], limit: usize, offset: usize, derived_only: bool) -> Value {
    let edges = citation_edges(rows);

    // Union-find over row indices.
    let mut parent: Vec<usize> = (0..rows.len()).collect();
    fn find(parent: &mut Vec<usize>, i: usize) -> usize {
        let mut root = i;
        while parent[root] != root {
            root = parent[root];
        }
        let mut cur = i;
        while parent[cur] != root {
            let next = parent[cur];
            parent[cur] = root;
            cur = next;
        }
        root
    }
    for &(citer, cited) in &edges {
        let (a, b) = (find(&mut parent, citer), find(&mut parent, cited));
        if a != b {
            parent[a] = b;
        }
    }

    let mut components: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..rows.len() {
        components.entry(find(&mut parent, i)).or_default().push(i);
    }

    // Rows arrive oldest-first, so members and chains are already in a
    // stable oldest-first order — deterministic pagination for free.
    let mut chains: Vec<Vec<usize>> = components
        .into_values()
        .filter(|members| members.len() >= 2)
        .filter(|members| !derived_only || members.iter().all(|&i| is_derived_note(&rows[i])))
        .collect();
    chains.sort_by_key(|members| members[0]);

    let total = chains.len();
    let page: Vec<Value> = chains
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|members| {
            let chain_edges: Vec<Value> = edges
                .iter()
                .filter(|(citer, cited)| members.contains(citer) && members.contains(cited))
                .map(|&(citer, cited)| json!({ "citer": rows[citer].id, "cited": rows[cited].id }))
                .collect();
            json!({
                "derived_only": members.iter().all(|&i| is_derived_note(&rows[i])),
                "rows": members.iter().map(|&i| public_row(&rows[i])).collect::<Vec<_>>(),
                "edges": chain_edges,
            })
        })
        .collect();

    json!({
        "kind": "cited",
        "scanned": rows.len(),
        "total": total,
        "offset": offset,
        "chains": page,
    })
}

/// Every `(citer, cited)` index pair where one row's content contains
/// another row's id verbatim. Ids are 10-char base62 (or legacy UUIDs),
/// so a hit is essentially proof of reference — the boundary check only
/// guards the theoretical id-inside-a-longer-token case.
pub(super) fn citation_edges(rows: &[Row]) -> Vec<(usize, usize)> {
    let mut edges = Vec::new();
    for (cited_idx, cited) in rows.iter().enumerate() {
        if cited.id.len() < 8 {
            continue; // defensive: too short to be an unambiguous citation
        }
        for (citer_idx, citer) in rows.iter().enumerate() {
            if citer_idx != cited_idx && contains_id(&citer.content, &cited.id) {
                edges.push((citer_idx, cited_idx));
            }
        }
    }
    edges
}

/// `content` contains `id` with non-alphanumeric characters (or string
/// ends) on both sides.
fn contains_id(content: &str, id: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = content[start..].find(id) {
        let abs = start + pos;
        let before_ok = abs == 0
            || !content[..abs]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric());
        let after_ok = !content[abs + id.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        start = abs + id.len();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::Origin;
    use crate::ops::chains::row;

    #[test]
    fn contains_id_respects_boundaries() {
        assert!(contains_id("see row 7gOUF6ryq6 for detail", "7gOUF6ryq6"));
        assert!(contains_id("(id=7gOUF6ryq6)", "7gOUF6ryq6"));
        assert!(!contains_id("x7gOUF6ryq6", "7gOUF6ryq6"));
        assert!(!contains_id("7gOUF6ryq6a", "7gOUF6ryq6"));
        assert!(contains_id("7gOUF6ryq6", "7gOUF6ryq6"));
    }

    #[test]
    fn cited_chain_found_and_derived_flagged() {
        let rows = vec![
            row(
                "aaaaaaaaaa",
                "design locked, impl not started",
                Origin::Derived,
            ),
            row(
                "bbbbbbbbbb",
                "build shipped (supersedes aaaaaaaaaa)",
                Origin::Derived,
            ),
            row("cccccccccc", "unrelated fact", Origin::User),
        ];
        let out = cited_chains(&rows, 10, 0, false);
        assert_eq!(out["total"], 1);
        let chain = &out["chains"][0];
        assert_eq!(chain["rows"].as_array().unwrap().len(), 2);
        assert_eq!(chain["derived_only"], true);
        assert_eq!(chain["edges"][0]["citer"], "bbbbbbbbbb");
        assert_eq!(chain["edges"][0]["cited"], "aaaaaaaaaa");
    }

    #[test]
    fn user_voice_row_breaks_derived_only() {
        let rows = vec![
            row("aaaaaaaaaa", "pref v1", Origin::User),
            row("bbbbbbbbbb", "updates aaaaaaaaaa", Origin::Derived),
        ];
        let out = cited_chains(&rows, 10, 0, false);
        assert_eq!(out["chains"][0]["derived_only"], false);
    }
}

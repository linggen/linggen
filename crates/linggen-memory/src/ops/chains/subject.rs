//! V2 scan: subject clusters over stored vectors.

use serde_json::{json, Value};
use std::collections::HashSet;

use super::cited::citation_edges;
use super::{is_derived_note, public_row};
use crate::memory::store::cosine_similarity;
use crate::memory::Memory as Row;
use crate::memory::Tier;

/// Subject clusters need members that genuinely share a subject, not
/// merely a domain — tighter than [`NEIGHBOR_FLOOR`], looser than the
/// restatement band (0.78+).
const SUBJECT_FLOOR: f32 = 0.70;
/// Quiet-subject gate: a cluster is digestible only when its NEWEST
/// member is at least this old. A subject still accreting rows is live —
/// digesting it would trade working detail for compression it doesn't
/// need yet. Age reduces resolution, never existence.
const SUBJECT_QUIET_DAYS: i64 = 30;
/// A 2-row near-pair is v1 (marker/cited) territory; digests earn their
/// keep at 3+.
const SUBJECT_MIN_ROWS: usize = 3;
/// Cap members per cluster — a digest over 15+ rows drifts toward the
/// forbidden mega state row; the tail stays for the next pass.
const SUBJECT_MAX_ROWS: usize = 12;

// ── V2: subject clusters ────────────────────────────────────────────────────

/// Star clusters over stored vectors: walk rows oldest-first; each
/// unassigned row seeds a cluster of the unassigned rows within
/// [`SUBJECT_FLOOR`] cosine of it (closest first, capped at
/// [`SUBJECT_MAX_ROWS`]). Seed-anchored on purpose — transitive closure
/// would chain drifting subjects into exactly the mega-blob the digest
/// rule forbids. Clusters below [`SUBJECT_MIN_ROWS`] dissolve (their
/// members stay available to later seeds).
///
/// Rows already in a cited chain are excluded — collapse provable
/// chains (v1) before digesting what remains. Rows a `subject` issue
/// names (`ruled`) leave the pool entirely: a rejected cluster is
/// recorded on ALL its member ids, so it can't re-form around a
/// neighboring seed and re-serve a settled ruling. Clusters whose newest
/// member is younger than [`SUBJECT_QUIET_DAYS`] are skipped: a live
/// subject keeps its detail.
pub(super) fn subject_clusters(
    rows: &[Row],
    limit: usize,
    offset: usize,
    derived_only: bool,
    ruled: &HashSet<String>,
    now: chrono::DateTime<chrono::Utc>,
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

    let mut ruled_skipped = 0usize;
    let pool: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(i, row)| {
            if ruled.contains(&row.id) {
                ruled_skipped += 1;
                return false;
            }
            !in_cited[*i]
                && row.tier != Tier::Core
                && row.vector.is_some()
                && (!derived_only || is_derived_note(row))
        })
        .map(|(i, _)| i)
        .collect();

    let mut assigned = vec![false; rows.len()];
    let mut clusters: Vec<(usize, Vec<(f32, usize)>)> = Vec::new(); // (seed, members-with-score)
    let mut live_skipped = 0usize;
    let quiet_cutoff = now - chrono::Duration::days(SUBJECT_QUIET_DAYS);

    for &seed in &pool {
        if assigned[seed] {
            continue;
        }
        let seed_vec = rows[seed]
            .vector
            .as_deref()
            .expect("pool rows have vectors");
        let mut members: Vec<(f32, usize)> = pool
            .iter()
            .filter(|&&i| i != seed && !assigned[i])
            .filter_map(|&i| {
                let v = rows[i].vector.as_deref()?;
                let score = cosine_similarity(seed_vec, v);
                (score >= SUBJECT_FLOOR).then_some((score, i))
            })
            .collect();
        if members.len() + 1 < SUBJECT_MIN_ROWS {
            continue; // seed stays unassigned — a later seed may absorb it
        }
        members.sort_by(|a, b| b.0.total_cmp(&a.0));
        members.truncate(SUBJECT_MAX_ROWS - 1);
        // Quiet-subject gate. A skipped live cluster still assigns its
        // members: any cluster containing a hot row fails this same gate,
        // so leaving hot rows loose would only let every later seed
        // rebuild the same doomed cluster (and inflate the skip count).
        assigned[seed] = true;
        for &(_, i) in &members {
            assigned[i] = true;
        }
        let newest = std::iter::once(seed)
            .chain(members.iter().map(|&(_, i)| i))
            .map(|i| rows[i].effective_timestamp())
            .max()
            .expect("cluster has at least the seed");
        if newest > quiet_cutoff {
            live_skipped += 1;
            continue;
        }
        members.insert(0, (1.0, seed));
        clusters.push((seed, members));
    }

    let total = clusters.len();
    let page: Vec<Value> = clusters
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(seed, members)| {
            json!({
                "seed_id": rows[seed].id,
                "derived_only": members.iter().all(|&(_, i)| is_derived_note(&rows[i])),
                "rows": members.iter().map(|&(score, i)| {
                    let mut row = public_row(&rows[i]);
                    row["score"] = json!((score * 1000.0).round() / 1000.0);
                    row
                }).collect::<Vec<_>>(),
            })
        })
        .collect();

    json!({
        "kind": "subject",
        "scanned": rows.len(),
        "total": total,
        "offset": offset,
        // Reported so a filtered scan never reads as a clean store:
        // rows already covered by a subject ruling (excluded from the
        // pool), and clusters still accreting rows (newest member inside
        // the quiet window).
        "ruled_skipped": ruled_skipped,
        "live_skipped": live_skipped,
        "clusters": page,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::Origin;
    use crate::ops::chains::row;

    fn vrow(id: &str, content: &str, origin: Origin, vector: Vec<f32>) -> Row {
        let mut m = row(id, content, origin);
        m.vector = Some(vector);
        m
    }

    /// A "now" far enough ahead of the fixtures' `created_at` (= real now)
    /// that every test cluster clears the quiet-subject gate.
    fn later() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now() + chrono::Duration::days(SUBJECT_QUIET_DAYS * 2)
    }

    #[test]
    fn subject_clusters_group_by_cosine_with_min_rows() {
        let rows = vec![
            vrow(
                "aaaaaaaaaa",
                "sanji planner one",
                Origin::Derived,
                vec![1.0, 0.0],
            ),
            vrow(
                "bbbbbbbbbb",
                "sanji planner two",
                Origin::Derived,
                vec![0.98, 0.199],
            ),
            vrow(
                "cccccccccc",
                "sanji planner three",
                Origin::Derived,
                vec![0.95, 0.312],
            ),
            vrow(
                "dddddddddd",
                "unrelated topic",
                Origin::Derived,
                vec![0.0, 1.0],
            ),
        ];
        let out = subject_clusters(&rows, 10, 0, true, &HashSet::new(), later());
        assert_eq!(out["total"], 1);
        let cluster = &out["clusters"][0];
        assert_eq!(cluster["rows"].as_array().unwrap().len(), 3);
        assert_eq!(cluster["seed_id"], "aaaaaaaaaa");
        assert_eq!(cluster["derived_only"], true);
    }

    #[test]
    fn subject_pair_below_min_rows_dissolves() {
        let rows = vec![
            vrow("aaaaaaaaaa", "topic a", Origin::Derived, vec![1.0, 0.0]),
            vrow(
                "bbbbbbbbbb",
                "topic a again",
                Origin::Derived,
                vec![0.99, 0.141],
            ),
            vrow("dddddddddd", "unrelated", Origin::Derived, vec![0.0, 1.0]),
        ];
        let out = subject_clusters(&rows, 10, 0, true, &HashSet::new(), later());
        assert_eq!(out["total"], 0);
    }

    #[test]
    fn subject_derived_only_excludes_user_rows_from_pool() {
        let rows = vec![
            vrow("aaaaaaaaaa", "pref one", Origin::User, vec![1.0, 0.0]),
            vrow("bbbbbbbbbb", "pref two", Origin::Derived, vec![0.98, 0.199]),
            vrow(
                "cccccccccc",
                "pref three",
                Origin::Derived,
                vec![0.95, 0.312],
            ),
        ];
        // With the user row excluded, only 2 remain — below min → no cluster.
        let out = subject_clusters(&rows, 10, 0, true, &HashSet::new(), later());
        assert_eq!(out["total"], 0);
        // Without derived_only, the mixed cluster forms and is flagged.
        let out = subject_clusters(&rows, 10, 0, false, &HashSet::new(), later());
        assert_eq!(out["total"], 1);
        assert_eq!(out["clusters"][0]["derived_only"], false);
    }

    #[test]
    fn live_subject_is_skipped_until_quiet() {
        // Fixture rows are created "now" → newest member is inside the
        // quiet window when judged from the real now.
        let rows = vec![
            vrow(
                "aaaaaaaaaa",
                "hot topic one",
                Origin::Derived,
                vec![1.0, 0.0],
            ),
            vrow(
                "bbbbbbbbbb",
                "hot topic two",
                Origin::Derived,
                vec![0.98, 0.199],
            ),
            vrow(
                "cccccccccc",
                "hot topic three",
                Origin::Derived,
                vec![0.95, 0.312],
            ),
        ];
        let out = subject_clusters(&rows, 10, 0, true, &HashSet::new(), chrono::Utc::now());
        assert_eq!(out["total"], 0);
        assert_eq!(out["live_skipped"], 1);
        // Once the subject has been quiet past the window, it serves.
        let out = subject_clusters(&rows, 10, 0, true, &HashSet::new(), later());
        assert_eq!(out["total"], 1);
        assert_eq!(out["live_skipped"], 0);
    }

    #[test]
    fn ruled_cluster_cannot_reform_around_a_neighbor() {
        let rows = vec![
            vrow(
                "aaaaaaaaaa",
                "ruled topic one",
                Origin::Derived,
                vec![1.0, 0.0],
            ),
            vrow(
                "bbbbbbbbbb",
                "ruled topic two",
                Origin::Derived,
                vec![0.98, 0.199],
            ),
            vrow(
                "cccccccccc",
                "ruled topic three",
                Origin::Derived,
                vec![0.95, 0.312],
            ),
        ];
        // A keep-separate ruling records ALL member ids — excluding only
        // the seed would let the same cluster re-form around a neighbor.
        let ruled: HashSet<String> = ["aaaaaaaaaa", "bbbbbbbbbb", "cccccccccc"]
            .map(String::from)
            .into();
        let out = subject_clusters(&rows, 10, 0, true, &ruled, later());
        assert_eq!(out["total"], 0);
        assert_eq!(out["ruled_skipped"], 3);
    }
}

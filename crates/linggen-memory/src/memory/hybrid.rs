//! Hybrid retrieval: rank by **semantic similarity lifted by keyword
//! matches**, so a query gets dense (cosine) relevance *plus* an
//! exact-keyword boost — without the keyword side ever inventing relevance
//! that isn't there.
//!
//! ## The score
//!
//! ```text
//! hybrid = cosine + keyword_boost
//! keyword_boost = W · (Σ idf of matched query terms) / (Σ idf of all query terms)
//! score  = hybrid × scope_weight        (ranks, and min_score gates it)
//! shown  = clamp01(score)
//! ```
//!
//! `scope_weight` is where the row's directory sits relative to the
//! session root ([`super::scope::Kin`]); `1.0` for a search with no root.
//! It multiplies the UNCLAMPED hybrid: clamping first would cap a row
//! weighed 0.7 at exactly 0.7, and a 0.7 floor could then never admit it.
//!
//! - **cosine** is the absolute dense-relevance signal (`[0,1]` in practice
//!   for Qwen3). It anchors the score: an unrelated row stays low.
//! - **keyword_boost** ∈ `[0, W]` is the IDF-weighted *fraction* of the
//!   query's words the row contains. Rare, high-signal words ("yinyue")
//!   dominate the weighting; a common word ("name", present in most rows)
//!   has near-zero IDF and so adds almost nothing. A row matching every
//!   query term gets the full `W`; a row matching nothing gets `0`.
//!
//! Both the **ordering** and the **displayed number** are this one score, so
//! the score is monotonic with rank *and* honest:
//!
//! - A keyword hit with mediocre cosine is lifted above non-matching rows
//!   (the "dog" / "yinyue" case) — fixing pure-keyword queries.
//! - An unrelated query never shows a fake `1.0`: with low cosine and no
//!   real keyword match the top row scores low. (The old RRF approach
//!   normalized to the result-set max, so its top was *always* 1.0 even when
//!   nothing matched — that is what this replaces.)
//! - A match on only a common word barely moves the score, so it can't
//!   inflate an otherwise-irrelevant row.
//!
//! ## Floor
//!
//! `min_score` gates the **weighted** score (not raw cosine). The default
//! (`recall_min_score`, 0.7) thus admits a keyword hit whose cosine alone
//! would fall under it — because the boost lifts it over the line — while
//! still dropping low-cosine rows with no real keyword match. The console
//! passes `min_score: 0` to show every match for inspection.
//!
//! ## Why in-process
//!
//! Both halves run over the live, flat-scanned candidate pool (the store has
//! no ANN/FTS index at this scale). Not LanceDB's native FTS: an FTS index
//! is not updated on append, so a freshly-written memory would be
//! unfindable by keyword until a reindex — the very bug this fixes. Revisit
//! at ~100k rows (same threshold the vector path and `list` flag).

use std::collections::HashSet;

use super::types::Memory;

/// Maximum lift a perfect lexical match (every query term present, weighted
/// by IDF) adds on top of cosine. Tuned so a real keyword hit clears the
/// recall floor and ranks above non-matches, while a match on only a common
/// (low-IDF) word adds almost nothing.
const KEYWORD_BOOST: f32 = 0.3;

/// A search candidate: a stored row plus its cosine similarity to the query
/// (already computed by the store from the row's vector; `0.0` for rows
/// with a null vector, which can still gain a keyword boost), and the scope
/// weight its directory earns in this session ([`super::scope::Kin`]; `1.0`
/// when the search has no root).
pub struct Candidate {
    pub memory: Memory,
    pub cosine: f32,
    pub weight: f32,
}

/// One ranked search hit.
#[derive(Debug, Clone)]
pub struct Hit {
    pub memory: Memory,
    /// Raw dense similarity.
    pub cosine: f32,
    /// `cosine + keyword_boost`, unclamped (can exceed 1.0).
    pub hybrid: f32,
    /// The row's scope weight in this session.
    pub weight: f32,
    /// `hybrid × weight`, unclamped: what `min_score` gates and rows rank by.
    pub score: f32,
}

impl Hit {
    /// The score shown to people and models: [`Self::score`] clamped to
    /// `[0, 1]`. Monotonic with rank.
    pub fn shown(&self) -> f32 {
        self.score.clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Script {
    Cjk,
    Word,
}

fn script(c: char) -> Option<Script> {
    if is_cjk(c) {
        Some(Script::Cjk)
    } else if c.is_alphanumeric() {
        Some(Script::Word)
    } else {
        None
    }
}

/// Han, kana, Hangul and their punctuation-free extensions: scripts written
/// without spaces, indexed as character bigrams.
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF   // Hiragana, Katakana
        | 0x3400..=0x4DBF // CJK Extension A
        | 0x4E00..=0x9FFF // CJK Unified
        | 0xAC00..=0xD7AF // Hangul syllables
        | 0xF900..=0xFAFF // CJK Compatibility
        | 0x20000..=0x2FA1F) // Extensions B–F, Compatibility Supplement
}

/// Lowercase, Unicode-aware tokenizer. A token is a run of one script:
/// Latin/digits (and other spaced scripts) split on anything else, so
/// `和move有` yields `move`; a CJK run becomes its character bigrams
/// (`有什么` → `有什`, `什么`), a lone CJK character itself. No stemming or
/// stopword list — IDF weighting already neutralizes common words.
fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut run: Vec<char> = Vec::new();
    let mut kind: Option<Script> = None;
    for c in text.chars().chain(std::iter::once(' ')) {
        let k = script(c);
        if k != kind || k.is_none() {
            flush(&mut run, kind, &mut out);
            kind = k;
        }
        if k.is_some() {
            run.extend(c.to_lowercase());
        }
    }
    out
}

fn flush(run: &mut Vec<char>, kind: Option<Script>, out: &mut Vec<String>) {
    match kind {
        Some(Script::Cjk) if run.len() > 1 => {
            out.extend(run.windows(2).map(|w| w.iter().collect()));
        }
        Some(_) if !run.is_empty() => out.push(run.iter().collect()),
        _ => {}
    }
    run.clear();
}

/// May this query term earn a keyword boost? Not when it is two ASCII
/// characters or fewer: `go`, `s`, `v2` match by chance (`GO` in a question
/// matched `no-go` in an unrelated row) and, being rare, would weigh most.
/// CJK bigrams are two characters but not ASCII, so they count.
fn boostable(term: &str) -> bool {
    !(term.is_ascii() && term.len() <= 2)
}

/// Inverse document frequency with `+1` smoothing — always positive, and a
/// term present in every doc lands near `0` (so common words barely count).
fn idf(df: usize, n: usize) -> f32 {
    (1.0 + (n as f32 - df as f32 + 0.5) / (df as f32 + 0.5)).ln()
}

/// IDF-weighted keyword boost per doc, scaled to `[0, KEYWORD_BOOST]`.
///
/// For each unique query term that actually occurs in the corpus (`df > 0`),
/// weight it by its IDF; a doc's boost is `W · (matched IDF mass / total IDF
/// mass)`. So matching the rare, meaningful word counts for most of the
/// boost and matching only a common word counts for almost none. Presence-
/// based: term frequency within a doc doesn't change the lift.
fn keyword_boosts(query_terms: &[String], docs: &[Vec<String>]) -> Vec<f32> {
    let n = docs.len();
    if n == 0 {
        return Vec::new();
    }

    // Unique query terms, then keep only those that occur in ≥1 doc and pair
    // each with its IDF weight. A term in no doc is unmatchable and must not
    // dilute the denominator.
    let mut terms: Vec<&str> = query_terms
        .iter()
        .map(String::as_str)
        .filter(|t| boostable(t))
        .collect();
    terms.sort();
    terms.dedup();
    let docs: Vec<HashSet<&str>> = docs
        .iter()
        .map(|d| d.iter().map(String::as_str).collect())
        .collect();
    let weighted: Vec<(&str, f32)> = terms
        .into_iter()
        .filter_map(|t| {
            let df = docs.iter().filter(|d| d.contains(t)).count();
            (df > 0).then(|| (t, idf(df, n)))
        })
        .collect();

    let idf_total: f32 = weighted.iter().map(|(_, w)| *w).sum();
    if idf_total <= 0.0 {
        return vec![0.0; n];
    }

    docs.iter()
        .map(|doc| {
            let matched: f32 = weighted
                .iter()
                .filter(|(t, _)| doc.contains(t))
                .map(|(_, w)| *w)
                .sum();
            KEYWORD_BOOST * (matched / idf_total)
        })
        .collect()
}

/// Score and rank `candidates`, returning the admitted rows ordered by
/// weighted relevance.
///
/// - `query_text` is tokenized for the keyword boost; with no terms (or no
///   corpus matches) the result degrades to pure-cosine ordering.
/// - Each row's score is `(cosine + boost) × weight`, unclamped, so a
///   weight below 1 still lets a strong match through and one above 1 still
///   separates two strong matches. `min_score` gates that score (`None` =
///   no floor); [`Hit::shown`] clamps it for display.
/// - Result is truncated to `limit`.
pub fn fuse(
    candidates: Vec<Candidate>,
    query_text: &str,
    limit: usize,
    min_score: Option<f32>,
) -> Vec<Hit> {
    if candidates.is_empty() || limit == 0 {
        return Vec::new();
    }

    let query_terms = tokenize(query_text);
    let docs: Vec<Vec<String>> = candidates
        .iter()
        .map(|c| tokenize(&c.memory.content))
        .collect();
    let boosts = keyword_boosts(&query_terms, &docs);

    let mut hits: Vec<Hit> = candidates
        .into_iter()
        .zip(boosts)
        .map(|(c, boost)| {
            let hybrid = (c.cosine + boost).max(0.0);
            Hit {
                score: hybrid * c.weight,
                memory: c.memory,
                cosine: c.cosine,
                hybrid,
                weight: c.weight,
            }
        })
        .filter(|h| min_score.is_none_or(|floor| h.score >= floor))
        .collect();

    // Order by score; tie-break by cosine so equal-score rows stay stable.
    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(b.cosine.total_cmp(&a.cosine))
    });
    hits.truncate(limit);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::types::{MemoryType, Origin};

    fn cand(content: &str, cosine: f32) -> Candidate {
        weighed(content, cosine, 1.0)
    }

    fn weighed(content: &str, cosine: f32, weight: f32) -> Candidate {
        Candidate {
            memory: Memory::new(content, MemoryType::Fact, Origin::User),
            cosine,
            weight,
        }
    }

    #[test]
    fn tokenize_splits_and_lowercases() {
        assert_eq!(tokenize("Yinyue's dog!"), vec!["yinyue", "s", "dog"]);
        assert!(tokenize("   ").is_empty());
    }

    #[test]
    fn tokenize_splits_at_script_boundaries() {
        assert_eq!(
            tokenize("Profitec GO 和move有什么不同?"),
            vec!["profitec", "go", "和", "move", "有什", "什么", "么不", "不同"]
        );
        assert_eq!(tokenize("咖啡机"), vec!["咖啡", "啡机"]);
        assert_eq!(tokenize("v2版本"), vec!["v2", "版本"]);
        assert_eq!(tokenize("灵境，九鼎录"), vec!["灵境", "九鼎", "鼎录"]);
    }

    #[test]
    fn chinese_glued_to_english_still_matches() {
        // "move" inside "和move有" used to be one token and match nothing.
        let cands = vec![
            cand("The Profitec Move has a bigger boiler", 0.50),
            cand("unrelated espresso note", 0.50),
        ];
        let out = fuse(cands, "和move有什么不同", 10, None);
        assert!(out[0].memory.content.contains("Move"));
        assert!(out[0].hybrid > 0.5, "{}", out[0].hybrid);
        assert_eq!(out[1].hybrid, 0.50);
    }

    #[test]
    fn short_ascii_terms_earn_no_boost() {
        // "GO" in the question matched "no-go" in a Sanji row by chance and,
        // being rare, took most of the boost.
        let cands = vec![
            cand("Camera-3D is a measured no-go", 0.45),
            cand("other row", 0.45),
        ];
        let out = fuse(cands, "GO", 10, None);
        assert!(out.iter().all(|h| h.hybrid == 0.45), "{out:?}");
        assert!(boostable("和"), "a lone CJK character still counts");
        assert!(boostable("不同"), "a CJK bigram still counts");
        assert!(!boostable("go") && !boostable("s") && boostable("dog"));
    }

    #[test]
    fn chinese_bigram_matches() {
        let cands = vec![cand("咖啡机放在厨房", 0.40), cand("厨房很小", 0.40)];
        let out = fuse(cands, "咖啡机", 10, None);
        assert!(out[0].memory.content.contains("咖啡"));
        assert!(out[0].hybrid > 0.6);
    }

    #[test]
    fn weight_multiplies_the_unclamped_hybrid() {
        // cosine 0.9 + full boost 0.3 = 1.2; × 0.7 = 0.84, which a 0.7 floor
        // admits. Clamping before weighing would cap it at 0.7 × 1.0 = 0.70.
        let out = fuse(
            vec![weighed("profitec move boiler", 0.9, 0.7)],
            "profitec move",
            10,
            Some(0.8),
        );
        assert_eq!(out.len(), 1);
        assert!((out[0].hybrid - 1.2).abs() < 1e-5, "{}", out[0].hybrid);
        assert!((out[0].score - 0.84).abs() < 1e-5, "{}", out[0].score);
    }

    #[test]
    fn min_score_gates_the_weighted_score() {
        let cands = vec![
            weighed("near row", 0.75, 0.8),   // 0.60
            weighed("same row", 0.62, 1.2),   // 0.744
            weighed("person row", 0.71, 1.0), // 0.71
        ];
        let out = fuse(cands, "", 10, Some(0.7));
        let got: Vec<&str> = out.iter().map(|h| h.memory.content.as_str()).collect();
        assert_eq!(got, ["same row", "person row"]);
    }

    #[test]
    fn shown_score_is_clamped_but_rank_is_not() {
        let cands = vec![
            weighed("a little less", 0.88, 1.2), // 1.056
            weighed("best", 0.95, 1.2),          // 1.14
        ];
        let out = fuse(cands, "", 10, None);
        assert_eq!(
            out[0].memory.content, "best",
            "ranked by the unclamped score"
        );
        assert_eq!(out[0].shown(), 1.0);
        assert_eq!(out[1].shown(), 1.0);
        assert!(out[0].score > 1.0 && out[0].hybrid == 0.95);
    }

    #[test]
    fn the_profitec_question_does_not_reach_a_sanji_row() {
        // Live case 2026-10-22: a Linggen session asked "Profitec GO 和move有
        // 什么不同?" and a Sanji row about a camera "no-go" came back.
        // Root ~/workspace/linggen vs ~/workspace/rust/sanji: far.
        let cosine = 0.457; // the row's real cosine on the live store
        let far = crate::memory::scope::Kin::Far.weight();
        let out = fuse(
            vec![
                weighed("Camera-3D is a measured no-go", cosine, far),
                weighed("Profitec Pro 600 manual", 0.2, 1.0),
            ],
            "Profitec GO 和move有什么不同?",
            10,
            Some(0.7),
        );
        assert!(out.is_empty(), "{out:?}");
        // Even unweighed, "GO" no longer lifts it over the floor.
        let out = fuse(
            vec![weighed("Camera-3D is a measured no-go", cosine, 1.0)],
            "Profitec GO 和move有什么不同?",
            10,
            None,
        );
        assert_eq!(out[0].hybrid, cosine);
    }

    #[test]
    fn keyword_hit_beats_higher_cosine_non_match() {
        // The motivating bug: a row that literally contains "dog" but has a
        // lower cosine must outrank higher-cosine rows that don't match.
        let cands = vec![
            cand("agents can ask the user a question", 0.58),
            cand("the responses api reasoning effort", 0.56),
            cand(
                "user has a male dog named Yinyue, separate from the cat",
                0.55,
            ),
        ];
        let out = fuse(cands, "dog", 10, Some(0.6));
        assert_eq!(
            out.len(),
            1,
            "only the boosted keyword hit clears the floor"
        );
        assert!(
            out[0].memory.content.contains("dog"),
            "the dog row ranks first"
        );
        assert!(out[0].score > 0.6, "its score is lifted over the floor");
    }

    #[test]
    fn floor_admits_lexical_hit_below_cosine_threshold() {
        // cosine 0.55 < floor 0.6, but the keyword boost lifts hybrid over it.
        let out = fuse(vec![cand("my dog Yinyue", 0.55)], "dog", 10, Some(0.6));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].cosine, 0.55, "carried cosine is unchanged");
        assert!(
            out[0].score >= 0.6,
            "hybrid (cosine + boost) clears the floor"
        );
    }

    #[test]
    fn floor_still_drops_low_cosine_non_matches() {
        let out = fuse(
            vec![cand("totally unrelated row", 0.40)],
            "dog",
            10,
            Some(0.6),
        );
        assert!(out.is_empty(), "no keyword match and below floor → dropped");
    }

    #[test]
    fn unrelated_query_does_not_score_top_as_one() {
        // The bug the user caught: with the old RRF max-normalization the top
        // row was always 1.0. Now an unmatched query just shows raw cosine.
        let cands = vec![cand("foo bar", 0.30), cand("baz qux", 0.22)];
        let out = fuse(cands, "zzz", 10, None);
        assert_eq!(out[0].memory.content, "foo bar", "highest cosine first");
        assert!(out[0].shown() < 0.5, "no fake 1.0 for an unrelated query");
        assert_eq!(out[0].shown(), 0.30, "score == cosine when nothing matches");
    }

    #[test]
    fn common_term_adds_little_boost() {
        // Query "yinyue name": "name" is in every row (low IDF), "yinyue" in
        // one (high IDF). The row with the rare word must win clearly; a
        // row matching only the common word barely moves.
        let mut cands = vec![cand("yinyue is the dog name", 0.50)];
        for _ in 0..4 {
            cands.push(cand("some other memory with a name", 0.50));
        }
        let out = fuse(cands, "yinyue name", 10, None);
        assert!(
            out[0].memory.content.contains("yinyue"),
            "rare-word row ranks first"
        );
        assert!(
            out[0].score > 0.7,
            "rare-word match earns most of the boost"
        );
        assert!(
            out[1].score < 0.6,
            "common-word-only match barely lifts above its cosine"
        );
    }

    #[test]
    fn empty_query_terms_degrade_to_cosine_order() {
        // Punctuation-only query → no terms → pure cosine ordering + floor.
        let cands = vec![cand("alpha", 0.9), cand("beta", 0.7), cand("gamma", 0.3)];
        let out = fuse(cands, "!!!", 10, Some(0.5));
        let got: Vec<&str> = out.iter().map(|h| h.memory.content.as_str()).collect();
        assert_eq!(
            got,
            vec!["alpha", "beta"],
            "cosine order, gamma floored out"
        );
    }
}

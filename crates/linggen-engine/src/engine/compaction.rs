//! The row-rewrite half of a session's compaction: where the one summary
//! row goes in the session file, and how the live tail after it is
//! shortened (`doc/shared-session-spec.md` § Compaction).
//!
//! The summary is written under a pseudo-sender ([`COMPACTION_SENDER`]) that
//! every member reads as a system note. It goes where the verbatim tail
//! begins; the rows before it stay in the file for the person to scroll back
//! to, but no member's thread reaches past the newest summary. When to
//! compact and the summarizer call are the server's (`chat::compact_rows`).

use crate::engine::AgentEngine;
use crate::state_fs::sessions::ChatMsg;

/// The pseudo-sender a compaction summary is kept under: context for every
/// member, never a line of dialogue.
pub const COMPACTION_SENDER: &str = "compaction";

/// The rows from the newest compaction summary on (all of them when none).
pub fn since_compaction(rows: &[ChatMsg]) -> &[ChatMsg] {
    let start = rows
        .iter()
        .rposition(|r| !r.is_observation && r.from_id == COMPACTION_SENDER)
        .unwrap_or(0);
    &rows[start..]
}

fn row_tokens(r: &ChatMsg) -> usize {
    AgentEngine::estimate_tokens_for_text(&r.content)
}

/// Where the verbatim tail begins: the newest rows within `tail_budget`
/// tokens, moved forward to the user's line that opens an exchange so no
/// turn is cut in half. `None` when nothing before it is left to summarize
/// beyond the newest summary itself.
pub fn tail_cut(rows: &[ChatMsg], tail_budget: usize) -> Option<usize> {
    let base = rows.len() - since_compaction(rows).len();
    let mut cut = rows.len();
    let mut kept = 0;
    while cut > base {
        let t = row_tokens(&rows[cut - 1]);
        if kept + t > tail_budget {
            break;
        }
        kept += t;
        cut -= 1;
    }
    let opens = |r: &ChatMsg| !r.is_observation && r.from_id == "user";
    while cut < rows.len() && !opens(&rows[cut]) {
        cut += 1;
    }
    let summarized = rows[base..cut.min(rows.len())]
        .iter()
        .filter(|r| r.from_id != COMPACTION_SENDER)
        .count();
    (cut < rows.len() && summarized > 0).then_some(cut)
}

/// The file with the summary row put in at `cut` — the rows before it stay
/// for the person; every member's thread starts at the summary.
pub fn with_summary(mut rows: Vec<ChatMsg>, cut: usize, summary: ChatMsg) -> Vec<ChatMsg> {
    let at = cut.min(rows.len());
    rows.insert(at, summary);
    rows
}

/// Where a summary goes when the ordinary tail budget cannot open a cut:
/// everything before the latest user line, so a trailing tool result that
/// is itself bigger than the budget stays in the tail and can be shortened.
fn summary_cut(rows: &[ChatMsg], tail_budget: usize) -> Option<usize> {
    if let Some(cut) = tail_cut(rows, tail_budget) {
        return Some(cut);
    }
    let base = rows.len() - since_compaction(rows).len();
    let last_user = rows
        .iter()
        .rposition(|r| !r.is_observation && r.from_id == "user")?;
    if last_user <= base {
        return None;
    }
    let summarized = rows[base..last_user]
        .iter()
        .filter(|r| r.from_id != COMPACTION_SENDER)
        .count();
    (summarized > 0).then_some(last_user)
}

fn live_tokens(rows: &[ChatMsg], from: usize) -> usize {
    rows.iter().skip(from).map(row_tokens).sum()
}

/// Shorten tool rows in the live tail until that tail fits in `budget`.
/// Spoken lines are left as written. Returns whether a body changed.
fn shrink_live_observations(rows: &mut [ChatMsg], from: usize, budget: usize) -> bool {
    let floor = 32usize;
    let mut changed = false;
    loop {
        let total = live_tokens(rows, from);
        if total <= budget {
            break;
        }
        let Some((idx, this)) = rows
            .iter()
            .enumerate()
            .skip(from)
            .filter(|(_, r)| r.is_observation)
            .map(|(i, r)| (i, row_tokens(r)))
            .filter(|(_, t)| *t > floor)
            .max_by_key(|(_, t)| *t)
        else {
            break;
        };
        let keep = this.saturating_sub(total - budget).max(floor);
        let next = AgentEngine::truncate_to_tokens(&rows[idx].content, keep);
        if next == rows[idx].content {
            break;
        }
        rows[idx].content = next;
        changed = true;
    }
    changed
}

/// The session file after this compaction pass. `summary` is inserted where
/// the verbatim tail begins; rows before it stay. Tool rows still in the
/// live tail are shortened until that tail fits in `live_budget`. No summary
/// and an already-fitting tail leaves the rows untouched.
pub(crate) fn commit_compaction(
    mut rows: Vec<ChatMsg>,
    summary: Option<String>,
    agent_id: &str,
    tail_budget: usize,
    live_budget: usize,
) -> Vec<ChatMsg> {
    let Some(summary) = summary.filter(|s| !s.is_empty()) else {
        return rows;
    };
    let Some(cut) = summary_cut(&rows, tail_budget) else {
        return rows;
    };
    let timestamp = rows.get(cut).map(|r| r.timestamp).unwrap_or(0);
    let row = ChatMsg {
        run_id: None,
        via: None,
        via_note: None,
        agent_id: agent_id.to_string(),
        from_id: COMPACTION_SENDER.to_string(),
        to_id: "user".to_string(),
        content: summary,
        timestamp,
        is_observation: false,
        client_id: None,
        failed: false,
        images: Vec::new(),
    };
    rows = with_summary(rows, cut, row);
    // The summary sits at `cut`. Only the tail after it is shortened; the
    // rows before it stay whole for scrollback.
    shrink_live_observations(&mut rows, cut, live_budget);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(from: &str, content: &str) -> ChatMsg {
        ChatMsg {
            run_id: None,
            via: None,
            via_note: None,
            agent_id: "ling".into(),
            from_id: from.into(),
            to_id: "user".into(),
            content: content.into(),
            timestamp: 0,
            is_observation: false,
            client_id: None,
            failed: false,
            images: Vec::new(),
        }
    }

    /// The tail keeps the newest rows within its budget, starting at a user
    /// line; nothing to summarize past the newest summary leaves it alone.
    #[test]
    fn the_tail_starts_at_an_exchange_and_never_reaches_past_the_summary() {
        let long = "x".repeat(400); // ~100 tokens
        let rows = vec![
            row("user", &long),
            row("ling", &long),
            row("user", &long),
            row("yinyue", &long),
            row("user", "now"),
        ];
        assert_eq!(
            tail_cut(&rows, 150),
            Some(4),
            "her reply fits but never opens the tail"
        );
        assert_eq!(tail_cut(&rows, 250), Some(2), "the user's line opens it");
        assert_eq!(
            tail_cut(&rows, 10_000),
            None,
            "everything fits: nothing to do"
        );
        let summarized = vec![
            row("user", &long),
            row(COMPACTION_SENDER, "S"),
            row("user", "now"),
        ];
        assert_eq!(
            tail_cut(&summarized, 0),
            None,
            "only the summary before the tail"
        );
    }

    /// The summary goes in where the tail begins; the rows before it stay
    /// in the file, and every member's thread starts at it.
    #[test]
    fn the_summary_goes_in_at_the_cut_and_the_thread_starts_there() {
        let rows = vec![
            row("user", "去临淄"),
            row("ling", "临淄到了。"),
            row("yinyue", "小心。"),
            row("user", "去碣石"),
        ];
        let out = with_summary(rows, 3, row(COMPACTION_SENDER, "S"));
        let froms: Vec<&str> = out.iter().map(|r| r.from_id.as_str()).collect();
        assert_eq!(froms, ["user", "ling", "yinyue", "compaction", "user"]);
        let thread: Vec<&str> = since_compaction(&out)
            .iter()
            .map(|r| r.content.as_str())
            .collect();
        assert_eq!(thread, ["S", "去碣石"]);
    }

    /// A trailing tool result bigger than the budget stays after the summary
    /// and is shortened until the live tail fits. The rows before the summary
    /// stay intact.
    #[test]
    fn a_summary_is_inserted_and_a_trailing_tool_result_is_shortened() {
        let long = "x".repeat(4000);
        let mut huge = row("system", &"y".repeat(80_000));
        huge.is_observation = true;
        let rows = vec![
            row("user", &long),
            row("ling", &long),
            row("user", "now"),
            huge,
        ];
        let out = commit_compaction(rows, Some("S".into()), "ling", 100, 500);
        let summary_at = out
            .iter()
            .position(|r| r.from_id == COMPACTION_SENDER)
            .expect("summary row");
        assert_eq!(out[0].content, long, "scrollback before the summary stays");
        assert!(out[summary_at].content.contains('S'));
        let obs = out.iter().find(|r| r.is_observation).expect("tool row");
        assert!(obs.content.contains("truncated to fit context"));
        assert!(obs.content.chars().count() < 80_000);
        let live: usize = out.iter().skip(summary_at).map(row_tokens).sum();
        assert!(live <= 500, "live tail {live} over budget");
    }

    /// Nothing to write and a tail that already fits does not rewrite the file.
    #[test]
    fn a_fitting_thread_is_left_unchanged() {
        let rows = vec![row("user", "hi"), row("ling", "hello")];
        let out = commit_compaction(rows.clone(), None, "ling", 1000, 1000);
        assert_eq!(out, rows);
    }
}

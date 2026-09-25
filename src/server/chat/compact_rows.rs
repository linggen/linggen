//! What `/compact` does to a session's file.
//!
//! The file is the session's shared record: in an app's chat it holds the
//! user's lines, the app agent's thread, the companion's lines and the rows
//! kept beside them (recall, tool output). Compacting folds only the
//! compacting agent's own span — the rows of its thread the summary now
//! stands for — into ONE summary row under a pseudo-sender no chat shows
//! and no other agent reads ([`COMPACTION_SENDER`]). Every other speaker's
//! row stays where it was, and every row written after the snapshot (a
//! guest's answer that landed while the summary was being written) is kept
//! as it is.

use crate::message::ChatMessage;
use crate::state_fs::sessions::ChatMsg;

/// The pseudo-sender a compaction summary is kept under: context for the
/// agent that compacted, never a line of dialogue.
pub(crate) const COMPACTION_SENDER: &str = "compaction";

/// The session's rows after `agent` compacted its thread.
///
/// `rows` is the file as it is now; the first `snapshot_len` of them were
/// there when compaction began. `tail` is the agent's thread after the
/// summary — kept verbatim — and tells where the summarized span ends: the
/// span is the agent's own rows (and system rows) before the earliest row
/// the tail still holds. The span's rows go; the summary takes the place of
/// the first of them. Nothing on file to fold: the rows come back as they are.
pub(crate) fn compacted(
    rows: Vec<ChatMsg>,
    snapshot_len: usize,
    agent: &str,
    tail: &[ChatMessage],
    summary: &str,
) -> Vec<ChatMsg> {
    let snapshot_len = snapshot_len.min(rows.len());
    let cut = tail_start(&rows[..snapshot_len], agent, tail);
    let in_span = |i: usize, r: &ChatMsg| i < cut && (r.agent_id == agent || r.from_id == "system");
    let Some(first) = rows.iter().enumerate().position(|(i, r)| in_span(i, r)) else {
        return rows;
    };
    let summary_row = ChatMsg {
        agent_id: agent.to_string(),
        from_id: COMPACTION_SENDER.to_string(),
        to_id: agent.to_string(),
        content: summary.to_string(),
        timestamp: rows[first].timestamp,
        is_observation: false,
    };
    let mut out = Vec::with_capacity(rows.len());
    for (i, r) in rows.into_iter().enumerate() {
        if i == first {
            out.push(summary_row.clone());
        }
        if !in_span(i, &r) {
            out.push(r);
        }
    }
    out
}

/// Index of the earliest file row the kept tail still holds — matched from
/// the end, in order, against the agent's own dialogue rows. A tail message
/// found nowhere (a side-lines note, a recall block, tool output) is only in
/// memory and skipped. A mismatch can only move the cut earlier, which keeps
/// more rows, never fewer. No match: the whole snapshot is the span.
fn tail_start(rows: &[ChatMsg], agent: &str, tail: &[ChatMessage]) -> usize {
    let own: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.agent_id == agent && !r.is_observation && r.from_id != "system")
        .map(|(i, _)| i)
        .collect();
    let mut searched = own.len();
    let mut earliest = rows.len();
    for m in tail.iter().rev() {
        if let Some(k) = (0..searched)
            .rev()
            .find(|&k| holds(m, &rows[own[k]], agent))
        {
            earliest = own[k];
            searched = k;
        }
    }
    earliest
}

/// Whether the thread message `m` is file row `r`, as the thread carries it:
/// verbatim, or labeled when another speaker's words were relayed to it.
fn holds(m: &ChatMessage, r: &ChatMsg, agent: &str) -> bool {
    if m.content == r.content {
        return true;
    }
    r.from_id != "user"
        && r.from_id != agent
        && m.content == super::with_sender_label(&r.from_id, &r.content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(agent: &str, from: &str, content: &str) -> ChatMsg {
        ChatMsg {
            agent_id: agent.into(),
            from_id: from.into(),
            to_id: "user".into(),
            content: content.into(),
            timestamp: 0,
            is_observation: false,
        }
    }

    fn shape(rows: &[ChatMsg]) -> Vec<String> {
        rows.iter()
            .map(|r| format!("{}/{}:{}", r.agent_id, r.from_id, r.content))
            .collect()
    }

    /// An app chat compacted by Ling: his own span folds into one hidden
    /// summary row; her lines, the user's line to her, and a row she wrote
    /// after the snapshot all stay as they were, where they were. Recall and
    /// system rows in the span go; nothing becomes a "user" row.
    #[test]
    fn only_the_agents_own_span_folds_and_every_other_speaker_stays() {
        let mut rows = vec![
            row("ling", "user", "去临淄"),
            row("ling", "memory-recall", "From memory: …"),
            row("ling", "ling", "临淄到了。"),
            row("yinyue", "user", "@银月 哪里有水边?"),
            row("yinyue", "yinyue", "山里有溪。"),
            row("ling", "system", "Tool Look: …"),
            row("ling", "user", "去碣石"),
            row("ling", "ling", "碣石到了。"),
        ];
        let snapshot = rows.len();
        rows.push(row("yinyue", "yinyue", "海风大。"));
        let tail = vec![
            ChatMessage::new("user", "[Said in this chat since your last turn]\n…"),
            ChatMessage::new("user", "去碣石"),
            ChatMessage::new("assistant", "碣石到了。"),
        ];
        let out = compacted(rows, snapshot, "ling", &tail, "- went to 临淄");
        assert_eq!(
            shape(&out),
            [
                "ling/compaction:- went to 临淄",
                "yinyue/user:@银月 哪里有水边?",
                "yinyue/yinyue:山里有溪。",
                "ling/user:去碣石",
                "ling/ling:碣石到了。",
                "yinyue/yinyue:海风大。",
            ]
        );
    }

    /// A relayed line sits in the thread labeled; it still marks the tail.
    #[test]
    fn a_relayed_row_in_the_tail_is_found_by_its_label() {
        let rows = vec![
            row("ling", "user", "old"),
            row("ling", "ling", "old reply"),
            row("ling", "yinyue", "Ling, the pass is open."),
            row("ling", "ling", "Then we go."),
        ];
        let tail = vec![
            ChatMessage::new("user", "[Yinyue]: Ling, the pass is open."),
            ChatMessage::new("assistant", "Then we go."),
        ];
        let out = compacted(rows, 4, "ling", &tail, "S");
        assert_eq!(
            shape(&out),
            [
                "ling/compaction:S",
                "ling/yinyue:Ling, the pass is open.",
                "ling/ling:Then we go.",
            ]
        );
    }

    /// Nothing of the agent's on file before the tail: the file is left
    /// exactly as it was.
    #[test]
    fn nothing_to_fold_leaves_the_file_alone() {
        let rows = vec![row("yinyue", "yinyue", "hi"), row("ling", "user", "q")];
        let tail = vec![ChatMessage::new("user", "q")];
        let out = compacted(rows.clone(), 2, "ling", &tail, "S");
        assert_eq!(shape(&out), shape(&rows));
    }
}

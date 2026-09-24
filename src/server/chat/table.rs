//! The table — an app chat's visible dialogue, as the companion reads it.
//!
//! In an app with a chat, the user, the app's agent and the companion share
//! one conversation. She reads all of it that the user can see — every line
//! the chat shows, and nothing it hides: no system prompt, no tool calls or
//! results, no hidden page kickoffs, no recalled memory rows. A long chat is
//! cut from the front to a budget, in whole chunks, so the thread keeps the
//! same beginning for many turns and the provider's prompt cache holds.

use crate::engine::agent::AgentManager;
use crate::message::ChatMessage;
use crate::state_fs::sessions::ChatMsg;
use std::collections::HashMap;
use std::path::Path;

/// About how many tokens of the table she reads.
const BUDGET_TOKENS: usize = 24_000;

/// Rows cut from the front together, so the thread's start moves rarely.
const CUT_CHUNK: usize = 32;

/// Whether the chat shows this row to the user. Mirrors the chat surface's
/// own rule (`shouldHideInternalChatMessage`), plus the rows the engine
/// keeps for itself: observations (tool calls and results) and "system".
pub(crate) fn shown_to_user(m: &ChatMsg) -> bool {
    let text = m.content.trim_start();
    !m.is_observation
        && m.from_id != "system"
        && !text.is_empty()
        && !m.content.contains("[HIDDEN]")
        && !text.starts_with("Starting autonomous loop for task:")
        && !text.starts_with("Used tool:")
        && !text.starts_with("Delegated task:")
        && !text.starts_with("{\"type\":")
        && !is_tool_line(text)
}

/// "Tool Bash: …" — a tool's output leaked into a row.
fn is_tool_line(text: &str) -> bool {
    let Some(rest) = text.strip_prefix("Tool ") else {
        return false;
    };
    let name_len = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    name_len > 0 && rest[name_len..].starts_with(':')
}

/// The newest rows that fit `budget`, cut from the front in whole chunks
/// of [`CUT_CHUNK`]: the cut only moves when the chat grows past a chunk,
/// so turn after turn the thread starts at the same row. A tail that still
/// overflows after chunked cuts is cut row by row, keeping at least the
/// newest row.
fn within_budget<T>(mut rows: Vec<T>, cost: impl Fn(&T) -> usize, budget: usize) -> Vec<T> {
    let costs: Vec<usize> = rows.iter().map(&cost).collect();
    let total: usize = costs.iter().sum();
    let mut cut = 0;
    let mut kept = total;
    while kept > budget && cut + CUT_CHUNK < rows.len() {
        kept -= costs[cut..cut + CUT_CHUNK].iter().sum::<usize>();
        cut += CUT_CHUNK;
    }
    while kept > budget && cut + 1 < rows.len() {
        kept -= costs[cut];
        cut += 1;
    }
    rows.drain(..cut);
    rows
}

fn token_cost(m: &ChatMsg) -> usize {
    crate::engine::AgentEngine::estimate_tokens_for_text(&m.content)
}

/// The rows of a chat that are dialogue: shown to the user, and spoken by
/// the user or an agent — never a pseudo-sender such as recalled memory.
fn dialogue(rows: Vec<ChatMsg>, is_agent: &HashMap<String, bool>) -> Vec<ChatMsg> {
    rows.into_iter()
        .filter(shown_to_user)
        .filter(|m| m.from_id == "user" || is_agent.get(&m.from_id).copied().unwrap_or(false))
        .collect()
}

/// The session's visible dialogue, oldest first, within the budget.
pub(crate) async fn read(manager: &AgentManager, root: &Path, session_id: &str) -> Vec<ChatMsg> {
    let rows = manager
        .global_sessions
        .get_chat_history(session_id)
        .unwrap_or_default();
    let root = root.to_path_buf();
    let mut is_agent: HashMap<String, bool> = HashMap::new();
    for m in &rows {
        if m.from_id != "user" && !is_agent.contains_key(&m.from_id) {
            let known = manager.agent_exists(&root, &m.from_id).await;
            is_agent.insert(m.from_id.clone(), known);
        }
    }
    within_budget(dialogue(rows, &is_agent), token_cost, BUDGET_TOKENS)
}

/// The dialogue as `reader`'s thread: its own lines are its replies; the
/// user's are the user's; another agent's are relayed, labeled with who
/// said them.
pub(crate) fn as_thread(rows: &[ChatMsg], reader: &str) -> Vec<ChatMessage> {
    rows.iter()
        .map(|m| {
            if m.from_id == reader {
                ChatMessage::new("assistant", m.content.clone())
            } else {
                ChatMessage::new("user", labeled(m))
            }
        })
        .collect()
}

/// The dialogue as one text to read: "[Speaker]: line", oldest first.
pub(crate) fn as_transcript(rows: &[ChatMsg]) -> String {
    rows.iter()
        .map(|m| match m.from_id.as_str() {
            "user" => format!("[User]: {}", m.content),
            _ => labeled(m),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A row as the model reads another speaker's line. Older relay rows carry
/// their label in the text already.
fn labeled(m: &ChatMsg) -> String {
    if m.from_id == "user" || m.content.starts_with('[') {
        return m.content.clone();
    }
    format!("[{}]: {}", super::sender_label(&m.from_id), m.content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(from: &str, content: &str, obs: bool) -> ChatMsg {
        ChatMsg {
            agent_id: "ling".into(),
            from_id: from.into(),
            to_id: "user".into(),
            content: content.into(),
            timestamp: 0,
            is_observation: obs,
        }
    }

    fn agents() -> HashMap<String, bool> {
        [("ling", true), ("yinyue", true), ("memory-recall", false)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect()
    }

    /// Everything the chat hides stays out; everything it shows is in, in
    /// order — the user's lines, the app agent's replies, hers.
    #[test]
    fn she_reads_what_the_chat_shows_and_nothing_it_hides() {
        let rows = vec![
            row("user", "[HIDDEN] [scene] opened", false),
            row("system", "Starting autonomous loop for task: x", true),
            row(
                "ling",
                "{\"type\":\"tool\",\"tool\":\"Look\",\"args\":{}}",
                true,
            ),
            row(
                "system",
                "Tool Look: Bash output (exit_code: Some(0))",
                true,
            ),
            row("memory-recall", "From memory (2 days ago): …", false),
            row("user", "去临淄", false),
            row("ling", "Tool Look: leaked", false),
            row("ling", "临淄到了。", false),
            row("user", "@银月 哪里有水边?", false),
            row("yinyue", "山里有溪。", false),
            row("user", "[HIDDEN] [scene] won haunt:jingwei", false),
            row("ling", "精卫不再挡路。", false),
        ];
        let seen: Vec<String> = dialogue(rows, &agents())
            .into_iter()
            .map(|m| format!("{}:{}", m.from_id, m.content))
            .collect();
        assert_eq!(
            seen,
            [
                "user:去临淄",
                "ling:临淄到了。",
                "user:@银月 哪里有水边?",
                "yinyue:山里有溪。",
                "ling:精卫不再挡路。",
            ]
        );
    }

    /// Her own lines are her replies; Ling's reach her labeled as his.
    #[test]
    fn as_her_thread_her_lines_are_hers_and_others_are_labeled() {
        let rows = vec![
            row("user", "去临淄", false),
            row("ling", "临淄到了。", false),
            row("yinyue", "山里有溪。", false),
        ];
        let thread = as_thread(&rows, "yinyue");
        let got: Vec<(&str, &str)> = thread
            .iter()
            .map(|m| (m.role.as_str(), m.content.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("user", "去临淄"),
                ("user", "[Ling]: 临淄到了。"),
                ("assistant", "山里有溪。"),
            ]
        );
        assert_eq!(
            as_transcript(&rows),
            "[User]: 去临淄\n[Ling]: 临淄到了。\n[Yinyue]: 山里有溪。"
        );
    }

    /// Under the budget nothing is cut; over it, whole chunks go from the
    /// front, so the start holds while the chat grows within a chunk.
    #[test]
    fn a_long_chat_is_cut_from_the_front_in_whole_chunks() {
        let rows: Vec<usize> = (0..100).collect();
        assert_eq!(within_budget(rows.clone(), |_| 1, 100).len(), 100);
        let kept = within_budget(rows.clone(), |_| 1, 50);
        assert_eq!(kept.first(), Some(&64), "cut two chunks of {CUT_CHUNK}");
        assert_eq!(kept.last(), Some(&99));
        // One more row: the start stays where it was.
        let grown: Vec<usize> = (0..101).collect();
        assert_eq!(within_budget(grown, |_| 1, 50).first(), Some(&64));
    }

    /// A tail that still overflows is cut row by row; the newest row stays.
    #[test]
    fn an_overflowing_tail_keeps_at_least_the_newest_row() {
        let rows: Vec<usize> = (0..10).collect();
        let kept = within_budget(rows, |_| 100, 250);
        assert_eq!(kept, vec![8, 9]);
        let one = within_budget(vec![7usize], |_| 1_000, 10);
        assert_eq!(one, vec![7]);
    }

    /// A compaction summary kept in the chat (the older part, folded) is a
    /// row like any other: it leads the dialogue, the full rows after it.
    #[test]
    fn a_folded_summary_leads_and_the_rows_after_it_follow_in_full() {
        let rows = vec![
            row("user", "- Went to 临淄; took the 榜文", false),
            row("user", "去碣石", false),
            row("ling", "碣石到了。", false),
        ];
        let seen = dialogue(rows, &agents());
        assert_eq!(seen.len(), 3);
        assert!(seen[0].content.starts_with("- Went to"));
    }
}

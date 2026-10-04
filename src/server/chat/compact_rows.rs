//! Compaction of a session's thread — one summary for every member.
//!
//! The thread is the session's (`chat::thread`), so its compaction is too
//! (`doc/shared-session-spec.md` § Compaction): one summary row, written into
//! the file under a pseudo-sender ([`COMPACTION_SENDER`]), that every member
//! reads as a system note. The summary goes where the verbatim tail begins;
//! the rows before it stay in the file for the person to scroll back to, but
//! no member's thread reaches past the newest summary.
//!
//! It runs before a turn when the thread outgrows the **smallest** member's
//! window × the session's threshold — so no member overflows on its turn —
//! and on `/compact`. The session's default member's model writes it.

use super::thread::since_compaction;
use super::ChatRunCtx;
use crate::engine::agent::AgentManager;
use crate::engine::AgentEngine;
use crate::state_fs::sessions::{ChatMsg, SessionMember};
use std::path::Path;

/// The pseudo-sender a compaction summary is kept under: context for every
/// member, never a line of dialogue.
pub(crate) const COMPACTION_SENDER: &str = "compaction";

/// A context window assumed for a model that names none.
const UNKNOWN_WINDOW: usize = 128_000;
/// The share of the smallest window the verbatim tail keeps.
const TAIL_SHARE: f64 = 0.15;
/// The default trigger fraction (`agent.compact_threshold` overrides it,
/// the session's own threshold over that).
const DEFAULT_THRESHOLD: f32 = 0.95;
/// A row's text in the transcript the summarizer reads, cut to this.
const ROW_CHARS: usize = 2000;

/// The thread size, in tokens, past which the session compacts: the
/// smallest member window × the threshold.
pub(crate) fn trigger_tokens(windows: &[Option<usize>], threshold: f32) -> usize {
    let smallest = smallest_window(windows);
    (smallest as f64 * threshold.clamp(0.1, 0.99) as f64) as usize
}

/// The smallest of the members' windows (an unknown one counts as
/// [`UNKNOWN_WINDOW`]).
fn smallest_window(windows: &[Option<usize>]) -> usize {
    windows
        .iter()
        .map(|w| w.unwrap_or(UNKNOWN_WINDOW))
        .min()
        .unwrap_or(UNKNOWN_WINDOW)
}

fn row_tokens(r: &ChatMsg) -> usize {
    AgentEngine::estimate_tokens_for_text(&r.content)
}

/// Where the verbatim tail begins: the newest rows within `tail_budget`
/// tokens, moved forward to the user's line that opens an exchange so no
/// turn is cut in half. `None` when nothing before it is left to summarize
/// beyond the newest summary itself.
pub(crate) fn tail_cut(rows: &[ChatMsg], tail_budget: usize) -> Option<usize> {
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

/// The rows a summary stands for, as one transcript: each line names its
/// speaker, so the summary keeps who said and did what.
pub(crate) fn transcript(rows: &[ChatMsg]) -> String {
    rows.iter()
        .filter(|r| !r.content.trim().is_empty())
        .map(|r| {
            let who = match (r.is_observation, r.from_id.as_str()) {
                (true, "system") => format!("tool result for {}", r.to_id),
                (true, from) => format!("{from} tool call"),
                (false, from) => from.to_string(),
            };
            format!("[{who}] {}", cut(&r.content, ROW_CHARS))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    format!("{head}...[truncated]")
}

/// The file with the summary row put in at `cut` — the rows before it stay
/// for the person; every member's thread starts at the summary.
pub(crate) fn with_summary(mut rows: Vec<ChatMsg>, cut: usize, summary: ChatMsg) -> Vec<ChatMsg> {
    let at = cut.min(rows.len());
    rows.insert(at, summary);
    rows
}

/// What the summarizer is asked.
fn summary_prompt(count: usize, transcript: &str, focus: Option<&str>) -> String {
    let focus = match focus.filter(|f| !f.is_empty()) {
        Some(f) => format!("\n\nIMPORTANT: Focus especially on: {f}\n"),
        None => String::new(),
    };
    format!(
        "You are summarizing a conversation between a user and one or more assistants \
         sharing one thread; each line names its speaker. The following {count} entries \
         are being compacted to free up context space.\n\n\
         Summarize them into a concise working state that preserves:\n\
         - What the user asked for, of whom, and current progress\n\
         - What each assistant said and did that still matters, under its name\n\
         - Key decisions made and why\n\
         - Files, tools and results that matter (with paths and names)\n\
         - Errors encountered and how they were resolved\n\
         - Pending work and the next step\n\n\
         Be concise but complete. Use bullet points.{focus}\n\n\
         --- ENTRIES TO SUMMARIZE ---\n{transcript}"
    )
}

/// The session's members' windows and its default member's model.
pub(crate) struct SessionModels {
    pub windows: Vec<Option<usize>>,
    pub summarizer: String,
    pub summarizer_id: String,
}

/// Read every member's window, and the default member's model to summarize
/// with — each member's model as it would run (`members::model_of`).
pub(crate) async fn session_models(
    manager: &AgentManager,
    root: &Path,
    members: &[SessionMember],
) -> Option<SessionModels> {
    let models = manager.models.read().await.clone();
    let mut windows = Vec::new();
    for m in members {
        let window = match super::members::model_of(manager, root, m).await {
            Some(id) => models.context_window(&id).await.ok().flatten(),
            None => None,
        };
        windows.push(window);
    }
    let summarizer_id = super::members::default_responder(members);
    let member = members.iter().find(|m| m.id == summarizer_id)?;
    let summarizer = super::members::model_of(manager, root, member).await?;
    Some(SessionModels {
        windows,
        summarizer,
        summarizer_id,
    })
}

/// Compact the session's thread into one summary row. Returns the summary
/// when one was written.
pub(crate) async fn compact_session(
    manager: &AgentManager,
    session_id: &str,
    models: &SessionModels,
    focus: Option<&str>,
) -> Option<String> {
    let store = &manager.global_sessions;
    let rows = store.get_chat_history(session_id).ok()?;
    let tail_budget = (smallest_window(&models.windows) as f64 * TAIL_SHARE) as usize;
    let cut = tail_cut(&rows, tail_budget)?;
    let base = rows.len() - since_compaction(&rows).len();
    let span = &rows[base..cut];
    let model_manager = manager.models.read().await.clone();
    let prompt = summary_prompt(span.len(), &transcript(span), focus);
    let (text, _) =
        crate::engine::summarize_text(&model_manager, &models.summarizer, (None, None), prompt)
            .await?;
    let summary = format!(
        "[Context compacted — {} messages summarized]\n\n{text}",
        span.len()
    );
    let row = ChatMsg {
        agent_id: models.summarizer_id.clone(),
        from_id: COMPACTION_SENDER.to_string(),
        to_id: "user".to_string(),
        content: summary.clone(),
        timestamp: rows[cut].timestamp,
        is_observation: false,
    };
    // The prefix up to `cut` is the snapshot's — rows written meanwhile
    // only append after it — so the summary goes in at the same place.
    if let Err(e) = store.edit_chat_history(session_id, |now| with_summary(now, cut, row)) {
        tracing::warn!("[compact] could not write the summary into {session_id}: {e}");
        return None;
    }
    tracing::info!(
        "[compact] {session_id}: {} rows summarized by {} ({})",
        span.len(),
        models.summarizer_id,
        models.summarizer
    );
    Some(summary)
}

/// Before a turn: when the speaking member's thread has outgrown the
/// smallest member's window × the session's threshold, compact the session
/// and rebuild the thread from the summary on. True when it compacted.
pub(super) async fn compact_if_due(engine: &mut AgentEngine, ctx: &ChatRunCtx) -> bool {
    let Some(sid) = ctx.session_id.as_deref() else {
        return false;
    };
    let members = super::members::of_session(&ctx.manager, sid).await;
    let Some(models) = session_models(&ctx.manager, &ctx.root, &members).await else {
        return false;
    };
    let threshold = engine
        .compact_threshold
        .or(engine.cfg.compact_threshold_default)
        .unwrap_or(DEFAULT_THRESHOLD);
    let limit = trigger_tokens(&models.windows, threshold);
    // The system prompt as last built (rebuilding it here would fetch the
    // core block again); a first turn counts the thread alone.
    let system = engine
        .cached_system_prompt
        .as_ref()
        .map_or(0, |c| AgentEngine::estimate_tokens_for_text(&c.content));
    let used = AgentEngine::estimate_tokens_for_messages(&engine.chat_history)
        + system
        + AgentEngine::estimate_tokens_for_text(&ctx.clean_msg);
    if used <= limit {
        return false;
    }
    tracing::info!("[compact] {sid}: thread ~{used} tokens over {limit} — compacting");
    let focus = engine.compact_focus.clone();
    if compact_session(&ctx.manager, sid, &models, focus.as_deref())
        .await
        .is_none()
    {
        return false;
    }
    super::thread::rebuild(engine, ctx).await;
    let _ = ctx.events_tx.send(crate::server::ServerEvent::StateUpdated);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(from: &str, content: &str) -> ChatMsg {
        ChatMsg {
            agent_id: "ling".into(),
            from_id: from.into(),
            to_id: "user".into(),
            content: content.into(),
            timestamp: 0,
            is_observation: false,
        }
    }

    /// The trigger is the smallest member's window × the threshold — so
    /// no member overflows on its turn; an unknown window counts as the
    /// default.
    #[test]
    fn the_trigger_is_the_smallest_members_window() {
        assert_eq!(trigger_tokens(&[Some(400_000), Some(100_000)], 0.5), 50_000);
        assert_eq!(trigger_tokens(&[Some(400_000)], 0.5), 200_000);
        assert_eq!(trigger_tokens(&[Some(400_000), None], 0.5), 64_000);
        assert_eq!(trigger_tokens(&[], 0.5), 64_000);
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

    /// The summarizer reads who said what — each member under its name.
    #[test]
    fn the_transcript_names_every_speaker() {
        let mut call = row("ling", r#"{"type":"tool","tool":"Look"}"#);
        call.is_observation = true;
        let t = transcript(&[row("user", "去临淄"), call, row("yinyue", "小心。")]);
        assert_eq!(
            t,
            "[user] 去临淄\n[ling tool call] {\"type\":\"tool\",\"tool\":\"Look\"}\n[yinyue] 小心。"
        );
    }
}

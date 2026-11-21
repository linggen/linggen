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
//! and on `/compact`. The session's default member's model writes it. Where
//! the summary row goes and how the tail is shortened is `engine::compaction`.

use crate::state_fs::RowAuthor;
use super::ChatRunCtx;
use crate::engine::agent::AgentManager;
use crate::engine::compaction::{since_compaction, tail_cut, with_summary, COMPACTION_SENDER};
use crate::engine::AgentEngine;
use crate::state_fs::sessions::{ChatMsg, SessionMember};
use std::path::Path;

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

/// The rows a summary stands for, as one transcript: each line names its
/// speaker, so the summary keeps who said and did what. Kept world blocks
/// are left out: readings as of a past moment, which the summary need not
/// repeat — the thread after it carries the newest.
pub(crate) fn transcript(rows: &[ChatMsg]) -> String {
    rows.iter()
        .filter(|r| !r.content.trim().is_empty())
        .filter(|r| !crate::engine::prompt::world::is_world_row(r))
        .map(|r| {
            let who = match (r.is_observation, r.author()) {
                (true, RowAuthor::System) => format!("tool result for {}", r.to_id),
                (true, from) => format!("{} tool call", from.id()),
                (false, from) => from.id().to_string(),
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
    asker: crate::engine::model_choice::Asker<'_>,
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
    let summarizer = within(&models, asker, super::members::model_of(manager, root, member).await?)?;
    Some(SessionModels {
        windows,
        summarizer,
        summarizer_id,
    })
}

/// The model `asker` may summarize with: the owner's is `model` as it is; a
/// guest's or a household person's one of the models they were given — the
/// summary is a call on this Mac's model, paid like a turn. `None` when they
/// were given none.
fn within(models: &crate::provider::models::ModelManager, asker: crate::engine::model_choice::Asker<'_>, model: String) -> Option<String> {
    use crate::engine::model_choice::{live, resolve, Asker, Rule};
    if matches!(asker, Asker::Owner) {
        return Some(model);
    }
    let rule = Rule { asker, may_switch: true };
    let choice = resolve(models, Some(&model), &model, rule);
    live(models, asker, choice.model()).ok()
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
    let mut spent = crate::engine::spend::CallSpend::default();
    let text = crate::engine::summarize_text(
        &model_manager,
        &models.summarizer,
        (None, None),
        prompt,
        &mut spent,
    )
    .await;
    // The summary is billed whether or not it came out usable.
    crate::engine::spend::note_summary(
        manager,
        &model_manager,
        session_id,
        &models.summarizer,
        &spent,
    );
    let text = text?;
    let summary = format!(
        "[Context compacted — {} messages summarized]\n\n{text}",
        span.len()
    );
    let row = ChatMsg {
        run_id: None,
        via: None,
        via_note: None,
        agent_id: models.summarizer_id.clone(),
        from_id: COMPACTION_SENDER.to_string(),
        to_id: "user".to_string(),
        content: summary.clone(),
        timestamp: rows[cut].timestamp,
        is_observation: false,
        client_id: None,
        failed: false,
        images: Vec::new(),
        place: None,
        heard: false,
    };
    let mut applied = false;
    let edited = store.edit_chat_history(session_id, |now| {
        let (rows, ok) = summary_onto(now, &rows[..cut], row);
        applied = ok;
        rows
    });
    if let Err(e) = edited {
        tracing::warn!("[compact] could not write the summary into {session_id}: {e}");
        return None;
    }
    if !applied {
        tracing::warn!("[compact] {session_id}: its history changed while the summary was written — summary dropped");
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

/// `now` with the summary of `made_from` in where `made_from` ends, and
/// whether it went in. Rows written meanwhile only append after it; when
/// the history no longer opens with `made_from` (a clear, another summary)
/// the summary is not of it, and `now` stays as it is.
fn summary_onto(now: Vec<ChatMsg>, made_from: &[ChatMsg], summary: ChatMsg) -> (Vec<ChatMsg>, bool) {
    if !now.starts_with(made_from) {
        return (now, false);
    }
    (with_summary(now, made_from.len(), summary), true)
}

/// Before a turn: when the speaking member's thread has outgrown the
/// smallest member's window × the session's threshold, compact the session
/// and rebuild the thread from the summary on. True when it compacted.
pub(super) async fn compact_if_due(engine: &mut AgentEngine, ctx: &ChatRunCtx) -> bool {
    let Some(sid) = ctx.session_id.as_deref() else {
        return false;
    };
    let members = super::members::of_session(&ctx.manager, sid).await;
    let shared = engine.guest_models();
    let asker = shared
        .as_deref()
        .map_or(crate::engine::model_choice::Asker::Owner, crate::engine::model_choice::Asker::Guest);
    let Some(models) = session_models(&ctx.manager, &ctx.root, &members, asker).await else {
        return false;
    };
    let threshold = engine
        .compact_threshold()
        .or(engine.cfg.compact_threshold_default)
        .unwrap_or(DEFAULT_THRESHOLD);
    let limit = trigger_tokens(&models.windows, threshold);
    // The system prompt as last built (rebuilding it here would fetch the
    // core block again); a first turn counts the thread alone.
    let system = engine
        .cached_system_prompt()
        .map_or(0, AgentEngine::estimate_tokens_for_text);
    let used = AgentEngine::estimate_tokens_for_messages(&engine.chat_history)
        + system
        + AgentEngine::estimate_tokens_for_text(&ctx.clean_msg);
    if used <= limit {
        return false;
    }
    tracing::info!("[compact] {sid}: thread ~{used} tokens over {limit} — compacting");
    let focus = engine.compact_focus().map(str::to_string);
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
            place: None,
            heard: false,
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

    /// The summarizer reads who said what — each member under its name.
    /// A summary goes only onto the history it was made from: rows added
    /// after it are fine, a history cleared or cut meanwhile drops it.
    #[test]
    fn a_summary_goes_only_onto_the_history_it_was_made_from() {
        let made_from = vec![row("user", "a"), row("ling", "b"), row("user", "c")];
        let summary = || row(COMPACTION_SENDER, "S");

        let mut grown = made_from.clone();
        grown.push(row("user", "d"));
        let (rows, ok) = summary_onto(grown, &made_from[..2], summary());
        assert!(ok);
        let texts: Vec<_> = rows.iter().map(|r| r.content.as_str()).collect();
        assert_eq!(texts, ["a", "b", "S", "c", "d"]);

        let cut_out = vec![row("user", "a"), row("user", "c")];
        let (rows, ok) = summary_onto(cut_out.clone(), &made_from[..2], summary());
        assert!(!ok, "a place's rows were cleared meanwhile");
        assert_eq!(rows, cut_out);

        let (rows, ok) = summary_onto(Vec::new(), &made_from[..2], summary());
        assert!(!ok && rows.is_empty(), "the session was cleared meanwhile");
    }

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

    /// Kept world blocks are readings as of a past moment: the summary
    /// leaves them behind with the span.
    #[test]
    fn the_transcript_leaves_old_world_blocks_out() {
        let world = row(crate::engine::prompt::world::WORLD_SENDER, "# World\n- disk");
        let t = transcript(&[row("user", "hi"), world, row("ling", "hello")]);
        assert_eq!(t, "[user] hi\n[ling] hello");
    }

    /// A summary is a call on this Mac's model, paid like a turn: the owner's
    /// is the model asked for as it is; a guest's or a household person's one
    /// of the models they were given, the first when theirs is not one of them;
    /// and none at all when they were given none.
    #[test]
    fn a_summary_runs_only_on_a_model_its_asker_was_given() {
        use crate::engine::model_choice::Asker;
        let models = crate::provider::models::ModelManager::new_with_credentials(
            vec![],
            &crate::credentials::Credentials::default(),
        );
        let given = vec!["gpt-6.1-terra".to_string()];
        let said = |asker, model: &str| within(&models, asker, model.to_string());
        assert_eq!(said(Asker::Owner, "gpt-6.2-sol"), Some("gpt-6.2-sol".to_string()));
        assert_eq!(said(Asker::Owner, "not-here"), Some("not-here".to_string()), "the owner's, as ever");
        assert_eq!(said(Asker::Guest(&given), "gpt-6.1-terra"), Some("gpt-6.1-terra".to_string()));
        assert_eq!(said(Asker::Guest(&given), "gpt-6.2-sol"), Some("gpt-6.1-terra".to_string()), "the member's own is not theirs");
        assert_eq!(said(Asker::Guest(&[]), "gpt-6.2-sol"), None, "given none");
    }
}

//! What Yinyue tells a person about Ling's work: one closed set of notices
//! and one entry, [`tell`], that every notice passes and that alone decides
//! (doc/yinyue-companion-spec.md § Herald):
//!
//! - **Whose.** A run in a session another tool started (an `agent_run`
//!   from Claude Code or Codex, `creator: external`) is never told — its
//!   caller has it — nor another person's.
//! - **Where.** Only while the person is away (`user_sees(Screen)`), and as
//!   her stage's cue alone: a notice never becomes a row of the thread. The
//!   thing it is about — the run's reply, the waiting question's card —
//!   carries one folded mark, "said while you were away", and only when it
//!   was said aloud.
//! - **The answer.** A wait's cue names its question, so her bubble carries
//!   that question's own card (the page's approve/deny), never a second one.
//! - **No repeat.** One telling per session while any question in it waits;
//!   a second question there is folded into it. Nothing about a wait is
//!   said or written once it is over.
//!
//! Other lines of hers — a mission run's fault (`mission_fact`), a restart's
//! notice, app moments, glances — are not notices of Ling's work and keep
//! their own paths.

use super::triggers::{ask_still_pending, error_announce_allowed, user_sees, Sight};
use super::*;
use std::collections::HashMap;
use std::sync::Mutex;

/// The closed set of notices about Ling's work.
pub(super) enum Herald {
    /// A run finished a reply while the person was away.
    ReplyReady {
        agent: String,
        session: Option<String>,
        run: Option<String>,
    },
    /// A run waits on the person: a question or a permission prompt.
    Waiting {
        agent: String,
        session: Option<String>,
        question_id: String,
    },
    /// A run failed.
    Failed {
        agent: String,
        session: Option<String>,
        run: Option<String>,
        auth: bool,
    },
}

impl Herald {
    fn agent(&self) -> &str {
        match self {
            Self::ReplyReady { agent, .. }
            | Self::Waiting { agent, .. }
            | Self::Failed { agent, .. } => agent,
        }
    }

    fn session(&self) -> Option<&str> {
        match self {
            Self::ReplyReady { session, .. }
            | Self::Waiting { session, .. }
            | Self::Failed { session, .. } => session.as_deref(),
        }
    }
}

/// The one entry of every notice about Ling's work.
pub(super) fn tell(state: &Arc<ServerState>, herald: Herald) {
    let state = state.clone();
    tokio::spawn(async move { tell_now(&state, herald).await });
}

async fn tell_now(state: &Arc<ServerState>, herald: Herald) {
    if !state.manager.agent_present(YINYUE_AGENT) {
        return; // not met: the page holds it, as it does for anyone
    }
    if herald.agent() == YINYUE_AGENT {
        // Her own failure is not news of Ling's work: it rides her next
        // woken turn (`run_fault`). Nothing else of hers is told.
        if let Herald::Failed { auth, .. } = herald {
            hold_her_own_failure(state, auth).await;
        }
        return;
    }
    // Whose: before anything is spent on it — a cooldown, a hold, a turn.
    if !whose(&state.manager, herald.session()).await {
        return;
    }
    // Where: only to a person who is away; at a screen the page holds it.
    if user_sees(state, Sight::Screen) {
        return;
    }
    match herald {
        Herald::ReplyReady {
            agent,
            session,
            run,
        } => {
            let kickoff = reply_kickoff(state, &agent, session.as_deref()).await;
            let about = About::Run { session, run };
            wake(state, (kickoff, Origin::Herald, "happy"), about).await;
        }
        Herald::Waiting { question_id, .. } => tell_wait(state, question_id).await,
        Herald::Failed {
            agent,
            session,
            run,
            auth,
        } => {
            // An error storm is one fact.
            if !error_announce_allowed() || !state.manager.get_config_snapshot().await.pet.enabled {
                return;
            }
            tracing::info!("[yinyue-watch] run by '{agent}' failed; Yinyue is handed the fact");
            // Said now or not at all: held, it would ride a later turn of
            // hers and land as a row — a notice is never one.
            let fact = super::run_fault::fact(&agent, auth, false);
            let about = About::Run { session, run };
            wake(state, (fact, Origin::Fault, "neutral"), about).await;
        }
    }
}

async fn hold_her_own_failure(state: &Arc<ServerState>, auth: bool) {
    if !error_announce_allowed() || !state.manager.get_config_snapshot().await.pet.enabled {
        return;
    }
    super::run_fault::hold(super::run_fault::fact(YINYUE_AGENT, auth, true));
}

/// Whether what happens in session `session_id` may be handed to the
/// builder's companion. The session is the builder's own
/// (`day_session::is_builders`, so never another person's words in the
/// builder's thread or voice), and not an external host's (it receives its
/// own results and answers its own prompts). Private to this file: no
/// notice is told but through [`tell`].
async fn whose(manager: &crate::engine::agent::AgentManager, session_id: Option<&str>) -> bool {
    let meta =
        session_id.and_then(|id| manager.global_sessions.get_session_meta(id).ok().flatten());
    !is_delegated(meta.as_ref())
        && crate::server::day_session::is_builders(manager, session_id).await
}

fn is_delegated(meta: Option<&crate::state_fs::sessions::SessionMeta>) -> bool {
    meta.is_some_and(|meta| meta.creator == "external")
}

/// What a notice is about: the thing that carries its mark.
enum About {
    /// A run's reply (or its failure line) in its session.
    Run {
        session: Option<String>,
        run: Option<String>,
    },
    /// A waiting question. `card`: her bubble carries its card — not for a
    /// question kept to a private app, whose words stay on its page.
    Wait { question_id: String, card: bool },
}

/// Wake her own turn for a notice and say what she chose to, if anything.
/// A fact of hers held for the person (`run_fault`) does not ride it: a
/// notice leaves no row behind, so it would be used up unrecorded.
async fn wake(
    state: &Arc<ServerState>,
    (kickoff, origin, emotion): (String, Origin, &str),
    about: About,
) -> Option<Said> {
    let reply = run_yinyue_turn(state, kickoff, origin).await;
    let said = reply.as_ref().and_then(Turned::finalize)?;
    if user_sees(state, Sight::Screen) {
        return None;
    }
    let said = say(state, said, emotion, &about);
    if let (About::Run { session, run }, Some(told)) = (&about, &said.told) {
        mark_reply(state, (session.clone(), run.clone()), told);
    }
    Some(said)
}

/// What a notice came to: her line, and the mark when it was heard.
struct Said {
    told: Option<Told>,
}

/// The one publication of a notice: her stage's cue, never a row. Heard —
/// said aloud to a surface that holds her — it is the mark's.
fn say(state: &Arc<ServerState>, said: Event, emotion: &str, about: &About) -> Said {
    let line = said.line;
    tracing::info!(
        "[yinyue-watch] Yinyue heralds ({} chars, {emotion})",
        line.text().len()
    );
    let card = match about {
        About::Wait {
            question_id,
            card: true,
            ..
        } => Some(question_id.clone()),
        _ => None,
    };
    let heard = crate::server::api::yinyue::emit_notice(state, line.clone(), emotion, card);
    let told = heard.then(|| Told {
        at: crate::util::now_ts_secs(),
        line: line.text().to_string(),
    });
    Said { told }
}

/// A notice said aloud, `at` (Unix seconds), with her line.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Told {
    pub at: u64,
    pub line: String,
}

/// The marks of notices said aloud, by what they are about. In memory: a
/// restart drops them, never the reply or the card itself.
static REPLIES: Mutex<Vec<((String, String), Told)>> = Mutex::new(Vec::new());
const REPLIES_KEPT: usize = 200;

/// The tellings of waits, one per waiting turn (`waits.rs`).
static WAITS: Mutex<Option<waits::Waits>> = Mutex::new(None);

fn with_waits<R>(f: impl FnOnce(&mut waits::Waits) -> R) -> R {
    let mut waits = WAITS.lock().unwrap_or_else(|p| p.into_inner());
    f(waits.get_or_insert_with(Default::default))
}

fn mark_reply(
    state: &Arc<ServerState>,
    (session, run): (Option<String>, Option<String>),
    told: &Told,
) {
    if let (Some(s), Some(r)) = (&session, &run) {
        let mut all = REPLIES.lock().unwrap_or_else(|p| p.into_inner());
        all.push(((s.clone(), r.clone()), told.clone()));
        let over = all.len().saturating_sub(REPLIES_KEPT);
        all.drain(..over);
    }
    send_mark(state, session, (run, None), told);
}

/// The live mark: one event per thing marked.
fn send_mark(
    state: &Arc<ServerState>,
    session: Option<String>,
    (run, question): (Option<String>, Option<String>),
    told: &Told,
) {
    let _ = state
        .events_tx
        .send(crate::engine::events::ServerEvent::NoticeTold {
            session_id: session,
            run_id: run,
            question_id: question,
            at: told.at,
            line: told.line.clone(),
        });
}

/// The mark on run `run`'s reply in `session`, when a notice of it was said
/// aloud — a page draws it on that run's last row.
pub(crate) fn reply_told(session: &str, run: &str) -> Option<Told> {
    let all = REPLIES.lock().unwrap_or_else(|p| p.into_inner());
    all.iter()
        .rev()
        .find(|((s, r), _)| s == session && r == run)
        .map(|(_, t)| t.clone())
}

/// The mark on waiting question `question_id`'s card: its own wait's
/// telling, when that was heard.
pub(crate) fn wait_told(question_id: &str) -> Option<Told> {
    with_waits(|w| w.told_for(question_id))
}

/// Every mark of `session` on a page's rows (each row's meta, in order): one
/// per told run, on that run's last row, as `told_aloud_at` and
/// `told_aloud_line`. Every history view calls this (the lock in `tests.rs`).
pub(crate) fn mark_rows<'a>(
    session: &str,
    metas: impl DoubleEndedIterator<Item = &'a mut serde_json::Value>,
) {
    let mut seen: Vec<String> = Vec::new();
    for meta in metas.rev() {
        let Some(run) = meta
            .get("run_id")
            .and_then(|r| r.as_str())
            .map(str::to_string)
        else {
            continue;
        };
        if seen.contains(&run) {
            continue;
        }
        if let Some(told) = reply_told(session, &run) {
            meta["told_aloud_at"] = told.at.into();
            meta["told_aloud_line"] = told.line.into();
        }
        seen.push(run);
    }
}

/// A waiting question as the herald reads it from the pending list.
struct Asked {
    agent: String,
    session: Option<String>,
    run: Option<String>,
    questions: Vec<crate::engine::tools::AskUserQuestion>,
}

/// Every question waiting now, and `question_id`'s own entry.
async fn pending_now(state: &Arc<ServerState>, question_id: &str) -> (Vec<String>, Option<Asked>) {
    let pending = state.pending_ask_user.lock().await;
    let asked = pending.get(question_id).map(|e| Asked {
        agent: e.agent_id.clone(),
        session: e.session_id.clone(),
        run: e.run_id.clone(),
        questions: e.questions.clone(),
    });
    (pending.keys().cloned().collect(), asked)
}

/// The wait a question belongs to: the root run of the turn tree that asked
/// it (a subagent's question is its turn's), else the question alone.
fn wait_key(
    manager: &crate::engine::agent::AgentManager,
    run: Option<&str>,
    question_id: &str,
) -> String {
    let Some(mut id) = run.map(str::to_string) else {
        return format!("q:{question_id}");
    };
    for _ in 0..32 {
        match manager.run_store.get_run(&id).and_then(|r| r.parent_run_id) {
            Some(parent) => id = parent,
            None => break,
        }
    }
    format!("run:{id}")
}

/// A run waits on the person: once per waiting turn, the question whole —
/// or, asked from a private app's turn, only that the app waits. A telling
/// that came to nothing hands the wait to its next question still waiting.
async fn tell_wait(state: &Arc<ServerState>, question_id: String) {
    let (pending, Some(asked)) = pending_now(state, &question_id).await else {
        return; // answered before she could be woken
    };
    let key = wait_key(&state.manager, asked.run.as_deref(), &question_id);
    let mut token = match with_waits(|w| w.claim(&key, &question_id, &pending)) {
        waits::Claim::Tell(token) => token,
        waits::Claim::Folded { told } => {
            tracing::info!("[yinyue-watch] ask {question_id} folded into its turn's telling");
            // Its card carries the wait's mark, live as on load.
            if let Some(told) = told {
                send_mark(
                    state,
                    asked.session.clone(),
                    (None, Some(question_id)),
                    &told,
                );
            }
            return;
        }
    };
    let (mut question_id, mut asked) = (question_id, asked);
    loop {
        if let Some(said) = tell_wait_once(state, &asked, &question_id).await {
            let marked = with_waits(|w| w.said(&key, token, said.told.clone()));
            if let Some(told) = said.told {
                for q in marked {
                    send_mark(state, asked.session.clone(), (None, Some(q)), &told);
                }
            }
            return;
        }
        let (pending, _) = pending_now(state, &question_id).await;
        let Some((next, next_token)) = with_waits(|w| w.came_to_nothing(&key, token, &pending))
        else {
            return;
        };
        let (_, Some(next_asked)) = pending_now(state, &next).await else {
            return;
        };
        (question_id, asked, token) = (next, next_asked, next_token);
    }
}

async fn tell_wait_once(
    state: &Arc<ServerState>,
    asked: &Asked,
    question_id: &str,
) -> Option<Said> {
    if user_sees(state, Sight::Screen) {
        return None;
    }
    let session = asked.session.as_deref();
    let kept = crate::engine::turn_place::question_kept_to(&state.manager, session).await;
    let kickoff = match &kept {
        Some(app) => kept_ask_kickoff(app),
        None => ask_kickoff(&asked.agent, question_id, &asked.questions),
    };
    let reply = herald_line(state, kickoff).await?;
    let said = reply.finalize()?;
    // Her turn can queue behind others; an answer meanwhile makes it stale.
    if !ask_still_pending(state, question_id).await {
        tracing::info!("[yinyue-watch] ask {question_id} answered while heralding — dropped");
        return None;
    }
    if user_sees(state, Sight::Screen) {
        tracing::info!("[yinyue-watch] user back at the screen — herald dropped");
        return None;
    }
    let about = About::Wait {
        question_id: question_id.to_string(),
        card: kept.is_none(),
    };
    Some(say(state, said, "neutral", &about))
}

/// Her reply to a wait's kickoff. A question waits inside a turn that holds
/// her session's turn lock when that session is the day's, so a full turn of
/// hers would wait for the very answer it is to call for: then she gives one
/// reply now, from home's view of the thread. Elsewhere her own turn runs.
/// Either way it is only her reply: nothing is shown or kept until it is
/// finalized and the question checked still waiting.
async fn herald_line(state: &Arc<ServerState>, kickoff: String) -> Option<Turned> {
    // The turn's own origin: every reading of its reply is by this value.
    let origin = Origin::Herald;
    let sid = super::session_roll::resolve_current_session(state).await;
    let free = state.manager.session_turn(&sid).try_lock().is_ok();
    if free {
        return run_yinyue_turn(state, kickoff, origin).await;
    }
    let ask = super::spoken::with_contract(kickoff, origin);
    // The person's language, set here as it is for every turn of hers.
    let language = super::turn::language_of_turn(state, &sid, origin, None).await;
    let text =
        crate::server::chat::reply_now(state.clone(), &sid, YINYUE_AGENT, (&ask, language)).await?;
    Some(Turned::of(text, origin))
}

/// Her kickoff for a finished reply: that it is ready, never what it did.
async fn reply_kickoff(state: &Arc<ServerState>, agent: &str, session: Option<&str>) -> String {
    // A notification, not a summary: she has NOT read the reply. The kickoff
    // carries the user's own request (for the topic) and says so, so all she
    // can announce is that a reply is ready — never what it did. Quoting
    // Ling's last words was tried and misfired both ways: a 300-char head
    // read as a reply that "broke off" (2026-09-01), and a content-free
    // "finished" once let her announce a deletion that never happened.
    let asked = match session {
        Some(sid) => super::triggers::last_user_line(state, sid).await,
        None => None,
    }
    .unwrap_or_default();
    let topic_block = if asked.is_empty() {
        String::new()
    } else {
        format!(" The user had asked: \"{asked}\".")
    };
    format!(
        "The agent \"{agent}\" just finished a reply while the user was away from \
         Linggen.{topic_block} You have NOT read the reply. If it's worth telling them \
         when they're back, say one brief line in your voice that their reply is ready \
         — name the topic in a few words taken from what they asked; never say what \
         the reply did, found, or changed, and never judge it. Right now says whether \
         they're back. If it's routine, say nothing. Never nag."
    )
}

fn ask_kickoff(
    agent_id: &str,
    question_id: &str,
    questions: &[crate::engine::tools::AskUserQuestion],
) -> String {
    let (summary, options) = ask_summary(questions);
    let opts = if options.is_empty() {
        String::new()
    } else {
        format!(" The options are: {options}.")
    };
    format!(
        "The agent \"{agent_id}\" is blocked, waiting on the user to answer: \
         \"{summary}\".{opts} The user has stepped away from the screen. Call them \
         back in your voice — one brief line. Its card waits on the page, and in your \
         bubble when it shows. When they give you their answer, relay \
         it with `answer_prompt` (question_id \"{question_id}\") — only their actual \
         words, never your own decision."
    )
}

/// The first question and its options, as her kickoff names them.
fn ask_summary(questions: &[crate::engine::tools::AskUserQuestion]) -> (String, String) {
    let q0 = questions.first();
    let summary = q0
        .map(|q| q.question.clone())
        .unwrap_or_else(|| "your input".to_string());
    let options = q0
        .map(|q| {
            q.options
                .iter()
                .map(|o| o.label.clone())
                .collect::<Vec<_>>()
                .join(" / ")
        })
        .unwrap_or_default();
    (summary, options)
}

/// Her kickoff for a question asked in the private app `app`: that it
/// waits, nothing of it.
fn kept_ask_kickoff(app: &str) -> String {
    format!(
        "The app \"{app}\" is waiting for the user to answer a question there; what it \
         asks is kept to that app. The user has stepped away from the screen. Call them back \
         to it in your voice — one brief line."
    )
}

mod waits;

#[cfg(test)]
mod tests;

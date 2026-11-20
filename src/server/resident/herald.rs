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
        questions: Vec<crate::engine::tools::AskUserQuestion>,
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
        Herald::Waiting {
            agent,
            session,
            question_id,
            questions,
        } => {
            tell_wait(state, (agent, session), question_id, questions).await;
        }
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
    Wait {
        session: Option<String>,
        question_id: String,
        card: bool,
    },
}

/// Wake her own turn for a notice and say what she chose to, if anything.
async fn wake(
    state: &Arc<ServerState>,
    (kickoff, origin, emotion): (String, Origin, &str),
    about: About,
) -> Option<Line> {
    // A failure of hers held for a turn that can carry it rides this one.
    let (kickoff, rode) = super::run_fault::ride(kickoff);
    let reply = run_yinyue_turn(state, kickoff, origin).await;
    let said = reply.as_ref().and_then(Turned::finalize);
    let line = match said {
        Some(said) if !user_sees(state, Sight::Screen) => Some(say(state, said, emotion, about)),
        _ => None,
    };
    if line.is_none() {
        if let Some(f) = rode {
            super::run_fault::hold(f);
        }
    }
    line
}

/// The one publication of a notice: her stage's cue, never a row — and,
/// when it was said aloud, the mark on the thing it is about.
fn say(state: &Arc<ServerState>, said: Event, emotion: &str, about: About) -> Line {
    let line = said.line;
    tracing::info!(
        "[yinyue-watch] Yinyue heralds ({} chars, {emotion})",
        line.text().len()
    );
    let card = match &about {
        About::Wait {
            question_id,
            card: true,
            ..
        } => Some(question_id.clone()),
        _ => None,
    };
    let aloud = crate::server::api::yinyue::emit_notice(state, line.clone(), emotion, card);
    if aloud {
        mark(state, about, line.text());
    }
    line
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

/// One telling per waiting session: the questions folded into it, and its
/// mark once said aloud.
struct Wait {
    questions: Vec<String>,
    told: Option<Told>,
}
static WAITS: Mutex<Option<HashMap<String, Wait>>> = Mutex::new(None);

fn mark(state: &Arc<ServerState>, about: About, line: &str) {
    let told = Told {
        at: crate::util::now_ts_secs(),
        line: line.to_string(),
    };
    let (session, run, question) = match about {
        About::Run { session, run } => {
            if let (Some(s), Some(r)) = (&session, &run) {
                let mut all = REPLIES.lock().unwrap_or_else(|p| p.into_inner());
                all.push(((s.clone(), r.clone()), told.clone()));
                let over = all.len().saturating_sub(REPLIES_KEPT);
                all.drain(..over);
            }
            (session, run, None)
        }
        About::Wait {
            session,
            question_id,
            ..
        } => {
            let mut waits = WAITS.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(wait) = waits
                .get_or_insert_with(HashMap::new)
                .get_mut(&wait_key(session.as_deref()))
            {
                wait.told = Some(told.clone());
            }
            (session, None, Some(question_id))
        }
    };
    let _ = state
        .events_tx
        .send(crate::engine::events::ServerEvent::NoticeTold {
            session_id: session,
            run_id: run,
            question_id: question,
            at: told.at,
            line: told.line,
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

/// The mark on waiting question `question_id`'s card: the telling of its
/// session's wait, when it was said aloud and the question is one of it.
pub(crate) fn wait_told(session: Option<&str>, question_id: &str) -> Option<Told> {
    let waits = WAITS.lock().unwrap_or_else(|p| p.into_inner());
    let wait = waits.as_ref()?.get(&wait_key(session))?;
    wait.questions
        .iter()
        .any(|q| q == question_id)
        .then(|| wait.told.clone())
        .flatten()
}

/// Every mark of `session` on a page's rows (each row's meta, in order): one
/// per told run, on that run's last row, as `told_aloud_at` and
/// `told_aloud_line`. The history views call this once each.
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

fn wait_key(session: Option<&str>) -> String {
    session.unwrap_or("").to_string()
}

/// Whether `question_id` opens a telling of its session's wait: true when
/// no question of an earlier telling there still waits. Otherwise it is
/// folded into that telling — told nothing new — and false.
fn claim_wait(session: Option<&str>, question_id: &str, pending: &[String]) -> bool {
    let mut waits = WAITS.lock().unwrap_or_else(|p| p.into_inner());
    let waits = waits.get_or_insert_with(HashMap::new);
    // A wait whose every question is answered is over: forgotten.
    waits.retain(|_, w| {
        w.questions
            .iter()
            .any(|q| pending.contains(q) && q != question_id)
    });
    let key = wait_key(session);
    if let Some(wait) = waits.get_mut(&key) {
        wait.questions.push(question_id.to_string());
        return false;
    }
    waits.insert(
        key,
        Wait {
            questions: vec![question_id.to_string()],
            told: None,
        },
    );
    true
}

/// A telling that came to nothing — silence, the answer came first, the
/// person came back — leaves no wait behind it: a later question there may
/// be told.
fn release_wait(session: Option<&str>) {
    let mut waits = WAITS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(waits) = waits.as_mut() {
        let key = wait_key(session);
        if waits.get(&key).is_some_and(|w| w.told.is_none()) {
            waits.remove(&key);
        }
    }
}

/// The question ids waiting now.
async fn pending_questions(state: &Arc<ServerState>) -> Vec<String> {
    state
        .pending_ask_user
        .lock()
        .await
        .keys()
        .cloned()
        .collect()
}

/// A run waits on the person: once per waiting session, the question whole
/// — or, asked from a private app's turn, only that the app waits.
async fn tell_wait(
    state: &Arc<ServerState>,
    (agent, session): (String, Option<String>),
    question_id: String,
    questions: Vec<crate::engine::tools::AskUserQuestion>,
) {
    let pending = pending_questions(state).await;
    if !pending.contains(&question_id) {
        return; // answered before she could be woken
    }
    if !claim_wait(session.as_deref(), &question_id, &pending) {
        tracing::info!("[yinyue-watch] ask {question_id} folded into its session's telling");
        return;
    }
    let told = tell_wait_once(
        state,
        (&agent, session.as_deref()),
        &question_id,
        &questions,
    )
    .await;
    if told.is_none() {
        release_wait(session.as_deref());
    }
}

async fn tell_wait_once(
    state: &Arc<ServerState>,
    (agent, session): (&str, Option<&str>),
    question_id: &str,
    questions: &[crate::engine::tools::AskUserQuestion],
) -> Option<Line> {
    let kept = crate::engine::turn_place::question_kept_to(&state.manager, session).await;
    let kickoff = match &kept {
        Some(app) => kept_ask_kickoff(app),
        None => ask_kickoff(agent, question_id, questions),
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
        session: session.map(str::to_string),
        question_id: question_id.to_string(),
        card: kept.is_none(),
    };
    Some(say(state, said, "neutral", about))
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

#[cfg(test)]
mod tests;

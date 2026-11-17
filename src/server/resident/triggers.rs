//! What wakes her: the event bus, mission runs that had failures, finished and failed runs,
//! pending asks, and where her words are delivered.

use super::guard::Source;
use super::*;

pub async fn yinyue_watch_loop(state: Arc<ServerState>) {
    let mut rx = state.events_tx.listen();
    tracing::info!("[yinyue-watch] started");
    loop {
        match rx.recv().await {
            Ok(event) => handle_event(&state, event),
            Err(RecvError::Lagged(n)) => {
                tracing::warn!("[yinyue-watch] lagged; skipped {n} events");
            }
            Err(RecvError::Closed) => break,
        }
    }
}

/// Cheap, synchronous classifier. Matches the coarse triggers and spawns any
/// async follow-up; every other event returns immediately.
pub(super) fn handle_event(state: &Arc<ServerState>, event: ServerEvent) {
    match event {
        ServerEvent::Notification(payload) => handle_notification(state, payload),
        // An agent parked on a question or a permission prompt — Yinyue heralds
        // "they need you". One hook covers both AskUser-tool questions and
        // permission approvals (the engine emits the same event for both).
        ServerEvent::AskUser {
            agent_id,
            question_id,
            questions,
            session_id,
        } => on_ask(state, agent_id, question_id, questions, session_id),
        // A peer agent sent a message. TO YINYUE → she receives it in her own
        // voice (herald). TO A CHAT AGENT (Ling, …) → it shows in that agent's
        // chat as a message from the sender, and that agent responds. Either way
        // the recipient's turn is tagged `agent_chat` (one-hop loop-break).
        ServerEvent::AgentChat {
            from,
            to,
            message,
            app,
            session_id,
        } => on_agent_chat(state, from, to, message, (app, session_id)),
        _ => {} // firehose + everything else dropped here, near-free
    }
}

/// A question waits: heralded whole — or, asked from a private app's turn,
/// only as that the app waits.
fn on_ask(
    state: &Arc<ServerState>,
    agent_id: String,
    question_id: String,
    questions: Vec<crate::engine::tools::AskUserQuestion>,
    session_id: Option<String>,
) {
    let state = state.clone();
    tokio::spawn(async move {
        match asked_in_private_app(&state, session_id.as_deref()).await {
            Some(app) => herald_kept_ask(&state, app, question_id, session_id),
            None => herald_ask(&state, agent_id, question_id, questions, session_id),
        }
    });
}

/// The private app a question in session `session_id` is asked from
/// (`turn_place::question_kept_to`, the one rule).
async fn asked_in_private_app(state: &Arc<ServerState>, session_id: Option<&str>) -> Option<String> {
    crate::engine::turn_place::question_kept_to(&state.manager, session_id).await
}

/// An agent parked on a question or a permission prompt — Yinyue heralds
/// "they need you" while the person is away.
fn herald_ask(
    state: &Arc<ServerState>,
    agent_id: String,
    question_id: String,
    questions: Vec<crate::engine::tools::AskUserQuestion>,
    session_id: Option<String>,
) {
    if delegated_session(state, session_id.as_deref()) {
        return; // the external host answers its own prompts
    }
    if agent_id == YINYUE_AGENT {
        return; // she's the one asking — not a herald
    }
    if !state.manager.agent_present(YINYUE_AGENT) {
        return; // not met: the prompt waits on the screen, as it does for anyone
    }
    if user_sees_screen(state) {
        return; // the question is on the screen they're looking at
    }
    let (summary, options) = ask_summary(&questions);
    let state = state.clone();
    tokio::spawn(async move {
        if !crate::server::day_session::is_builders(&state.manager, session_id.as_deref()).await {
            return; // another person's question: never in the builder's thread or voice
        }
        let opts = if options.is_empty() {
            String::new()
        } else {
            format!(" The options are: {options}.")
        };
        let kickoff = format!(
            "The agent \"{agent_id}\" is blocked, waiting on the user to answer: \
             \"{summary}\".{opts} The user has stepped away from the screen. Call them \
             back in your voice — one brief line, spoken aloud, plain prose. When they give you their answer, relay \
             it with `answer_prompt` (question_id \"{question_id}\") — only their actual \
             words, never your own decision. If it truly doesn't warrant interrupting now, \
             reply with exactly SILENT."
        );
        // Her turn can queue behind other Yinyue runs; if the user
        // answers the prompt meanwhile, announcing it is stale noise.
        // Check before spending a turn and again right before speaking.
        if !ask_still_pending(&state, &question_id).await {
            return;
        }
        let Some(reply) = herald_line(&state, kickoff).await else {
            return;
        };
        let Some(line) = spoken_line(&reply) else {
            tracing::info!("[yinyue-watch] Yinyue chose silence");
            return;
        };
        if !ask_still_pending(&state, &question_id).await {
            tracing::info!(
                "[yinyue-watch] ask {question_id} answered while heralding — dropped"
            );
            return;
        }
        if user_sees_screen(&state) {
            tracing::info!("[yinyue-watch] user back at the screen — herald dropped");
            return;
        }
        tracing::info!(
            "[yinyue-watch] Yinyue heralds ({} chars, neutral)",
            line.len()
        );
        crate::server::api::yinyue::emit_speak(&state, line, Some("neutral".to_string()), None);
    });
}

/// A question asked in a private app's turn: she calls the person back to
/// that app, told only that it waits for an answer — never the question,
/// its options or anything said there; her line is made of that alone.
fn herald_kept_ask(state: &Arc<ServerState>, app: String, question_id: String, session_id: Option<String>) {
    if delegated_session(state, session_id.as_deref()) || !state.manager.agent_present(YINYUE_AGENT) {
        return;
    }
    if user_sees_screen(state) {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        if !crate::server::day_session::is_builders(&state.manager, session_id.as_deref()).await {
            return;
        }
        if !ask_still_pending(&state, &question_id).await {
            return;
        }
        let Some(reply) = herald_line(&state, kept_ask_kickoff(&app)).await else {
            return;
        };
        let Some(line) = spoken_line(&reply) else {
            return;
        };
        if !ask_still_pending(&state, &question_id).await || user_sees_screen(&state) {
            return;
        }
        crate::server::api::yinyue::emit_speak(&state, line, Some("neutral".to_string()), None);
    });
}

/// Her kickoff for a question asked in the private app `app`: that it
/// waits, nothing of it.
fn kept_ask_kickoff(app: &str) -> String {
    format!(
        "The app \"{app}\" is waiting for the user to answer a question there; what it \
         asks is kept to that app. The user has stepped away from the screen. Call them back \
         to it in your voice — one brief line, spoken aloud, plain prose. If it truly \
         doesn't warrant interrupting now, reply with exactly SILENT."
    )
}

/// Her words for a herald. A question waits inside a turn that holds her
/// session's turn lock when that session is the day's, so a full turn of
/// hers would wait for the very answer it is to call for: then she gives one
/// line now, from home's view of the thread, and it is kept as her row once
/// the session is free. Elsewhere her own turn runs, as before.
async fn herald_line(state: &Arc<ServerState>, kickoff: String) -> Option<String> {
    let sid = super::session_roll::resolve_current_session(state).await;
    let free = state.manager.session_turn(&sid).try_lock().is_ok();
    if free {
        return run_yinyue_turn(state, kickoff, "event").await;
    }
    let ask = super::spoken::with_contract(kickoff, "event");
    let line = crate::server::chat::line_now(state.clone(), &sid, YINYUE_AGENT, &ask).await?;
    keep_when_free(state.clone(), sid, line.clone());
    Some(line)
}

/// Keep her herald line as her row in `sid` once its running turn is over.
fn keep_when_free(state: Arc<ServerState>, sid: String, line: String) {
    tokio::spawn(async move {
        let _turn = state.manager.session_turn(&sid).lock_owned().await;
        crate::server::chat::helpers::persist_and_emit_to_store(
            &state.manager.global_sessions,
            &state.events_tx,
            YINYUE_AGENT,
            YINYUE_AGENT,
            "user",
            &line,
            Some(&sid),
            false,
            None,
            None,
            None,
        )
        .await;
    });
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

/// A peer agent's message: to her, she answers it; to a chat agent, it
/// lands in that agent's chat.
fn on_agent_chat(
    state: &Arc<ServerState>,
    from: String,
    to: String,
    message: String,
    (app, sent_in): (Option<String>, Option<String>),
) {
    if from == to {
        return; // no self-send (also guarded in the tool)
    }
    let state = state.clone();
    if to == YINYUE_AGENT {
        if !state.manager.agent_present(YINYUE_AGENT) {
            return; // not met: nobody here to receive it
        }
        tokio::spawn(async move {
            let kickoff = format!(
                "The agent \"{from}\" sent you a message, addressed to YOU: \"{message}\". \
                 Respond however fits — it's yours to act on:\n\
                 • a nudge to move (dance, wave, nod, a little cheer…) → do it with Express;\n\
                 • news worth telling the user → say one brief line in your voice (spoken, \
                 plain prose), reading the room from Right now;\n\
                 • you can do both — move and speak;\n\
                 • if nothing fits, reply with exactly SILENT.\n\
                 You're reached via agent_chat, so you can't pass it to a third agent."
            );
            if let Some(reply) = run_yinyue_turn(&state, kickoff, "agent_chat").await {
                if let Some(line) = spoken_line(&reply) {
                    tracing::info!(
                        "[yinyue-watch] relays agent_chat ({} chars)",
                        line.len()
                    );
                    crate::server::api::yinyue::emit_speak(
                        &state,
                        line,
                        Some("neutral".to_string()),
                        None,
                    );
                } else {
                    tracing::info!("[yinyue-watch] chose silence (agent_chat)");
                }
            }
        });
    } else {
        tokio::spawn(async move {
            deliver_to_chat_agent(state, from, to, message, (app, sent_in)).await;
        });
    }
}

/// Dispatch a notification payload to its reaction.
pub(super) fn handle_notification(state: &Arc<ServerState>, payload: NotificationPayload) {
    if !state.manager.agent_present(YINYUE_AGENT) {
        return; // heralds are hers; before she is met there is no one to speak them
    }
    match payload {
        // A background mission finished. Only an unattended run that had
        // something go wrong reaches her, with what its record says — the
        // facts, not a verdict. A clean run hands her nothing: the record and
        // the missions page hold it, and a run that went as planned is not news.
        NotificationPayload::MissionCompleted {
            mission_id,
            mission_name,
            status,
            run_id,
            ..
        } => {
            // Guard: never react to a Yinyue-shaped mission (belt-and-suspenders).
            if mission_id.starts_with(YINYUE_AGENT) {
                return;
            }
            // A cancel is user-initiated — they know.
            if status == "cancelled" {
                return;
            }
            let run = mission_fact::Held {
                mission_id,
                label: mission_name,
                run_id,
            };
            mission_fact::hand_or_hold(state, run);
        }
        // A run errored — Yinyue surfaces it in her own voice. A DETERMINISTIC
        // line over the speak spine, never an LLM wake: the failure may be the
        // model backend itself, so waking her agent to announce it could fail too
        // (and loop). This INCLUDES her own turns failing (e.g. her model 400s) —
        // the announce is a fixed TTS line, not a new run, so it can't loop, and a
        // silent failure is exactly what leaves the user wondering what's wrong.
        // Rate-limited so an error storm doesn't make her chant.
        NotificationPayload::RunFailed {
            agent_id,
            auth_required,
            ..
        } => {
            if !error_announce_allowed() {
                return; // an error storm shouldn't make her repeat herself
            }
            // Gate on the Pet master switch (async read), then surface it.
            let state = state.clone();
            tokio::spawn(async move {
                if !state.manager.get_config_snapshot().await.pet.enabled {
                    return;
                }
                tracing::info!("[yinyue-watch] run by '{agent_id}' failed; Yinyue surfaces it");
                // An auth failure has a fix the user can act on — say the fix,
                // not a vague apology a signed-out install would repeat forever.
                let line = if auth_required {
                    "I can't reach my mind right now — it needs a sign-in over in Settings."
                        .to_string()
                } else {
                    error_line()
                };
                crate::server::api::yinyue::emit_speak(&state, line, Some("sad".to_string()), None);
            });
        }
        // A run finished cleanly. This fires on every reply, so only herald when
        // the user has stepped away — otherwise they already see it on screen.
        NotificationPayload::RunCompleted {
            agent_id,
            session_id,
        } => {
            if delegated_session(state, session_id.as_deref()) {
                return; // the external host receives its own result
            }
            if agent_id == YINYUE_AGENT {
                return; // never herald her own turns
            }
            if user_sees_screen(state) {
                return; // they're here; the reply is already on screen
            }
            let state = state.clone();
            tokio::spawn(async move {
                // A notification, not a summary: she has NOT read the reply.
                // The kickoff carries the user's own request (for the topic)
                // and says so, so all she can announce is that a reply is
                // ready — never what it did. Quoting Ling's last words was
                // tried and misfired both ways: a 300-char head read as a
                // reply that "broke off" (2026-09-01), and a content-free
                // "finished" once let her announce a deletion that never
                // happened. "They need you" is the AskUser hook's job.
                let asked = match session_id.as_deref() {
                    Some(sid) => last_user_line(&state, sid).await,
                    None => None,
                }
                .unwrap_or_default();
                let topic_block = if asked.is_empty() {
                    String::new()
                } else {
                    format!(" The user had asked: \"{asked}\".")
                };
                let kickoff = format!(
                    "The agent \"{agent_id}\" just finished a reply while the user was away from \
                     Linggen.{topic_block} You have NOT read the reply. If it's worth telling them \
                     when they're back, say one brief line in your voice that their reply is ready \
                     — name the topic in a few words taken from what they asked; never say what \
                     the reply did, found, or changed, and never judge it. Right now says whether \
                     they're back. If it's routine, reply with exactly SILENT. Spoken aloud: plain \
                     prose, no markdown. Never nag."
                );
                wake_herald(state, kickoff, "happy").await;
            });
        }
    }
}

/// External delegation sessions belong to the calling host, not the person.
fn delegated_session(state: &Arc<ServerState>, session_id: Option<&str>) -> bool {
    let meta = session_id.and_then(|id| {
        state
            .manager
            .global_sessions
            .get_session_meta(id)
            .ok()
            .flatten()
    });
    is_delegated(meta.as_ref())
}

fn is_delegated(meta: Option<&crate::state_fs::sessions::SessionMeta>) -> bool {
    meta.is_some_and(|meta| meta.creator == "external")
}

/// The user's most recent message in a session — the topic of the reply
/// Yinyue announces — trimmed to a short quote. Hidden prompts and tool
/// payloads are skipped. `None` when the session has nothing quotable, and
/// for a private app's session: its words are never quoted to her.
pub(super) async fn last_user_line(state: &Arc<ServerState>, session_id: &str) -> Option<String> {
    let place = crate::server::chat::turn_place::of_session_id(&state.manager, session_id);
    let day_on = crate::server::day_session::enabled(&state.manager).await;
    if place.private_app(&*state.manager.skills, day_on).await.is_some() {
        return None;
    }
    let history = state.manager.global_sessions.get_chat_history(session_id).ok()?;
    let line = history
        .iter()
        .rev()
        .filter(|m| m.from_id == "user")
        .map(|m| m.content.trim())
        .find(|c| !c.is_empty() && !c.starts_with('{') && !c.contains("[HIDDEN]"))?;
    let mut out: String = line.chars().take(80).collect();
    if line.chars().count() > 80 {
        out.push('…');
    }
    Some(out)
}

/// Min seconds between error announcements. Several runs can fail within seconds
/// (a flaky model backend), and she should mention it once, not chant.
pub(super) const ERROR_ANNOUNCE_COOLDOWN_SECS: u64 = 90;
pub(super) static LAST_ERROR_ANNOUNCE: AtomicU64 = AtomicU64::new(0);

/// True at most once per cooldown window; stamps the clock when it returns true.
pub(super) fn error_announce_allowed() -> bool {
    let now = crate::util::now_ts_secs();
    let last = LAST_ERROR_ANNOUNCE.load(Ordering::Relaxed);
    if now.saturating_sub(last) < ERROR_ANNOUNCE_COOLDOWN_SECS {
        return false;
    }
    LAST_ERROR_ANNOUNCE.store(now, Ordering::Relaxed);
    true
}

/// A brief, in-character "something's wrong" — her voice, never the raw error,
/// never jargon. Rotated so repeated failures don't read like a canned alert.
pub(super) fn error_line() -> String {
    const LINES: [&str; 4] = [
        "I'm sorry — something just went wrong behind the scenes.",
        "Something stumbled just now. I'm sorry to bring it up.",
        "That didn't go through cleanly — my apologies. I'm keeping an eye on it.",
        "Something's off just now, and I wanted you to know.",
    ];
    let idx = (crate::util::now_ts_secs() as usize) % LINES.len();
    LINES[idx].to_string()
}

/// Whether the user is at the screen right now. A herald of what they can
/// already see — a finished reply, a question with its buttons — is noise: she
/// speaks only when they have stepped away.
pub(super) fn user_sees_screen(state: &Arc<ServerState>) -> bool {
    state
        .manager
        .presence_snapshot()
        .state(crate::util::now_ts_secs())
        != "away"
}

/// Whether an AskUser question (or permission prompt) is still awaiting the
/// user. Answering — via the UI or `answer_prompt` — removes it from the map.
pub(super) async fn ask_still_pending(state: &Arc<ServerState>, question_id: &str) -> bool {
    state
        .pending_ask_user
        .lock()
        .await
        .contains_key(question_id)
}

/// Wake Yinyue to herald a worker event — a finished mission/run, or an agent
/// blocked on the user. She reads the room (the Right now block) and decides whether it's
/// worth a word; `SILENT` means say nothing (the never-nag discipline).
/// The line when she actually spoke — a caller that bought silence with a
/// notice needs to know whether anyone heard it.
pub(crate) async fn wake_herald(
    state: Arc<ServerState>,
    kickoff: String,
    emotion: &str,
) -> Option<String> {
    wake_and_speak(state, (kickoff, None), emotion, "event", Reach::Open, None).await
}

/// What woke a fault's turn: a mission run that went wrong.
pub(super) const FAULT_SOURCE: &str = "fault";

/// What woke a moment's turn from the app the person is using.
const IN_USE_SOURCE: &str = "event-in-use";

/// The trigger source of an app moment's turn: asked for, from the app the
/// person is using, or neither.
pub(crate) fn moment_source(asked: bool, in_use: bool) -> &'static str {
    match (asked, in_use) {
        (true, _) => "asked",
        (false, true) => IN_USE_SOURCE,
        (false, false) => "event",
    }
}

/// The app chat a moment's turn runs at: its session, and the app the
/// moment came from — the turn's place in a day's session, where every app
/// is at the one table (`server::day_session`).
#[derive(Clone, Debug)]
pub(crate) struct Table {
    pub session: String,
    pub app: Option<String>,
}

/// Wake her to tell a mission run's fault. Unprompted: quiet hours make it
/// wait, and the hourly ceiling neither counts nor holds it ([`Source::Fault`]).
pub(super) async fn wake_fault(state: Arc<ServerState>, fact: String) -> Option<String> {
    wake_and_speak(
        state,
        (fact, None),
        "neutral",
        FAULT_SOURCE,
        Reach::Open,
        None,
    )
    .await
}

/// Wake her for an app moment; `source` is [`moment_source`]. `asked`: the
/// user asked her for this — the same spoken final paragraph, under the
/// contract that offers no SILENT. Sealed either way: her line is the whole
/// of her answer (`Reach::Sealed`). `table`: the app chat the moment names —
/// her turn runs there.
pub(crate) async fn wake_for_moment(
    state: Arc<ServerState>,
    moment: (String, Option<Table>),
    emotion: &str,
    source: &str,
    language: Option<String>,
) -> Option<String> {
    wake_and_speak(state, moment, emotion, source, Reach::Sealed, language).await
}

/// Wake her, and speak her line. Returns the line she said aloud — `None`
/// when the run failed, she produced nothing, or chose silence.
pub(super) async fn wake_and_speak(
    state: Arc<ServerState>,
    (kickoff, table): (String, Option<Table>),
    emotion: &str,
    trigger_source: &str,
    reach: Reach,
    language: Option<String>,
) -> Option<String> {
    let said_in = table.as_ref().map(|t| t.session.clone());
    // `None`: the run failed or she produced nothing.
    let reply = match reach {
        Reach::Open => run_yinyue_turn(&state, kickoff, trigger_source).await,
        Reach::Sealed => run_moment_turn(&state, (kickoff, table), trigger_source, language).await,
        Reach::SpeakOnly => {
            super::turn::run_home(&state, kickoff, trigger_source, reach, language).await
        }
    }?;
    let Some(line) = spoken_line(&reply) else {
        tracing::info!("[yinyue-watch] Yinyue chose silence");
        return None;
    };
    if let Some(source) = unprompted_source(reach, trigger_source) {
        if !passes_last_word(&state, source).await {
            return None;
        }
        if source.counts() {
            super::guard::spoke_unprompted();
        }
    }
    tracing::info!(
        "[yinyue-watch] Yinyue heralds ({} chars, {emotion})",
        line.len()
    );
    crate::server::api::yinyue::emit_speak(&state, line.clone(), Some(emotion.to_string()), said_in.as_deref());
    Some(line)
}

/// What woke an unprompted turn of hers — `None` for a reply: her ordinary
/// turns (a person's words, a herald of a run they started) and anything
/// asked. Only these are under the guardrails.
pub(super) fn unprompted_source(reach: Reach, trigger_source: &str) -> Option<Source> {
    if trigger_source == "asked" {
        return None;
    }
    match reach {
        Reach::Open => (trigger_source == FAULT_SOURCE).then_some(Source::Fault),
        Reach::Sealed if trigger_source == IN_USE_SOURCE => Some(Source::InUse),
        Reach::Sealed => Some(Source::Moment),
        Reach::SpeakOnly if trigger_source == super::look_again::LOOK_AGAIN_SOURCE => {
            Some(Source::Note)
        }
        Reach::SpeakOnly => Some(Source::Glance),
    }
}

/// The last word before her voice: checked before the turn too, this is for
/// two unprompted turns that ran at once. A fault is never dropped here: it
/// waited for its hour before the turn, and a line that comes out as quiet
/// hours begin is still hers to say.
async fn passes_last_word(state: &Arc<ServerState>, source: Source) -> bool {
    if source == Source::Fault {
        return true;
    }
    let pet = state.manager.get_config_snapshot().await.pet;
    let Some(held) = super::guard::held_now(&pet, source) else {
        return true;
    };
    let item = super::guard::Withheld {
        what: "a line".to_string(),
        source: source.name(),
        key: format!("line-{}", crate::util::now_ts_secs()),
    };
    super::guard::record_held(&item, &held);
    false
}

/// Deliver an `agent_chat` message to a CHAT agent (Ling, …): show it in that
/// agent's most-recent session as a message from the sender, then run the
/// agent's turn so it responds in the chat. The session is marked
/// `agent_chat`-triggered, so that turn can't relay onward (the loop-break).
pub(super) async fn deliver_to_chat_agent(
    state: Arc<ServerState>,
    from: String,
    to: String,
    message: String,
    (app, sent_in): (Option<String>, Option<String>),
) {
    // With an explicit app target, deliver into that app's (skill's) session so
    // the recipient runs with that app's tools. Otherwise prefer the session the
    // user is viewing; else the agent's latest.
    let mut candidates = Vec::new();
    // An app that keeps a session of its own is delivered there, never in a
    // day (`server::day_session::takes`).
    let day = match sent_in.filter(|sid| crate::server::day_session::is_day(sid)) {
        // Sent from a day: delivered in that day, its person's.
        Some(day) => crate::server::day_session::takes(&state.manager, app.as_deref())
            .await
            .then_some(day),
        None => crate::server::day_session::day_for(&state.manager, None, app.as_deref()).await,
    };
    if let Some(day) = day.as_deref() {
        // Linggen's daily session: the table they share, in the app's place.
        candidates.extend(session_and_root(&state, day));
    } else if let Some(skill) = app.as_deref() {
        if let Ok(sessions) = state.manager.global_sessions.list_sessions() {
            let mut sessions: Vec<_> = sessions
                .into_iter()
                .filter(|m| m.skill.as_deref() == Some(skill))
                .collect();
            sessions.sort_by_key(|m| std::cmp::Reverse(m.created_at));
            candidates.extend(
                sessions
                    .into_iter()
                    .map(|m| (m.id, std::path::PathBuf::from(m.cwd))),
            );
        }
    } else {
        candidates.extend(focused_session(&state));
        candidates.extend(latest_session_for_agent(&state, &to));
    }
    let selected = select_handoff(candidates, |sid| {
        if !handoff_eligible(&state, sid) {
            return None;
        }
        let held = state.manager.session_turn(sid).try_lock_owned().ok()?;
        handoff_eligible(&state, sid).then_some(held)
    });
    let (session_id, root, _turn) = match selected {
        Some(selected) => selected,
        None => {
            // The day's session busy: the message waits for its turn there.
            let waits = day.as_deref().and_then(|d| session_and_root(&state, d));
            let made = match waits {
                Some(found) => Some(found),
                None => new_handoff_session(&state, app.as_deref(), &to).await,
            };
            let Some((sid, root)) = made else {
                return;
            };
            let held = state.manager.session_turn(&sid).lock_owned().await;
            (sid, root, held)
        }
    };
    let agent = match state
        .manager
        .get_or_create_session_agent(&session_id, &root, &to)
        .await
    {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("[agent-chat] could not load '{to}': {e}");
            return;
        }
    };
    // A message addressed to an agent seats it where it lands.
    crate::server::chat::members::seat(&state.manager, &session_id, &to).await;
    // No run is begun here: the turn core tracks its own; one around it was
    // a second "running" row per message, and the stop button's pick could
    // be the one the engine never checks.
    // Show the incoming message in the chat, attributed to the sender. The
    // attribution is the message's from_id — the chat surfaces render the
    // "[Yinyue]" label from that one fact, and the model gets the same label
    // via ChatRunCtx::labeled_msg. The content itself stays clean.
    // Past a restart's snapshot it waits in the plan instead; kept here, its
    // session's pending turn is in the snapshot.
    let incoming = crate::server::admission::Incoming {
        session: &session_id,
        agent: &to,
        text: &message,
        images: &[],
        sender: Some(&from),
    };
    let again = || {
        let sent_in = Some(session_id.clone());
        resend_agent_chat(state.clone(), (&from, &to, &message), (app.clone(), sent_in))
    };
    // admit can capture the message in a frozen restart plan, so validate
    // before calling it, while this session's turn place is still held.
    if !handoff_eligible(&state, &session_id) {
        tracing::warn!("[agent-chat] session became ineligible at admission");
        return;
    }
    let gate = match crate::server::admission::admit(&state.admission, incoming, again).await {
        Ok(Some(gate)) => gate,
        Ok(None) => return,
        Err(error) => {
            tracing::error!("[agent-chat] '{from}'→'{to}' not kept: {error}");
            return;
        }
    };
    // Recheck while the admission gate and turn lock are held.
    if !handoff_eligible(&state, &session_id) {
        tracing::warn!("[agent-chat] session became ineligible at admission");
        return;
    }
    crate::server::chat::helpers::persist_and_emit_message(
        &state.manager,
        &state.events_tx,
        &root,
        &to,
        &from,
        &to,
        &message,
        Some(&session_id),
        false,
        None,
    )
    .await;
    drop(gate);

    {
        let mut engine = agent.lock().await;
        // Loop-break: this turn was reached via agent_chat → it can't agent_chat
        // onward. Mark/clear INSIDE the engine lock so the flag's lifetime matches
        // exactly the turn that owns the engine — a concurrent turn on the same
        // session can't observe or clear another turn's mark.
        state.manager.mark_agent_chat_session(&session_id);
        engine.set_parent_agent(None);
        engine.last_assistant_text = None;
        let ctx = crate::server::chat::ChatRunCtx {
            via: None,
            state: state.clone(),
            manager: state.manager.clone(),
            events_tx: state.events_tx.clone(),
            root: root.clone(),
            agent_id: to.clone(),
            session_id: Some(session_id.clone()),
            clean_msg: message,
            images: Vec::new(),
            policy: crate::engine::session_policy::SessionPolicy::owner(),
            sender: Some(from.clone()),
            silence_ok: false,
            place: delivered_place(&state, &session_id, app.as_deref()),
        };
        crate::server::chat::run_session_turn(&ctx, &mut engine, &state.manager, None).await;
        state.manager.clear_agent_chat_session(&session_id);
    }

    tracing::info!("[agent-chat] delivered '{from}'→'{to}' in {session_id}");
}

/// A session's id and folder, when it is there.
fn session_and_root(
    state: &Arc<ServerState>,
    session_id: &str,
) -> Option<(String, std::path::PathBuf)> {
    let meta = state.manager.global_sessions.get_session_meta(session_id).ok()??;
    Some((meta.id, std::path::PathBuf::from(meta.cwd)))
}

/// Where a delivered message's turn stands: in the day's session, the app
/// it names (home when none); anywhere else, the session's own place.
fn delivered_place(
    state: &Arc<ServerState>,
    session_id: &str,
    app: Option<&str>,
) -> crate::engine::turn_place::TurnPlace {
    if !crate::state_fs::day_session::is_day_session(session_id) {
        return crate::server::chat::turn_place::of_session_id(&state.manager, session_id);
    }
    let meta = state.manager.global_sessions.get_session_meta(session_id).ok().flatten();
    crate::engine::turn_place::TurnPlace::of_request(app, meta.as_ref())
}

/// The session the user is currently viewing (`set_view_context`), if any — so
/// agent_chat lands in the chat they have open.
pub(super) fn focused_session(state: &Arc<ServerState>) -> Option<(String, std::path::PathBuf)> {
    let (sid, root) = state.current_view.lock_ok().clone()?;
    if sid.is_empty() {
        return None;
    }
    let root = if root.is_empty() {
        crate::util::resolve_path(std::path::Path::new("~/.linggen"))
    } else {
        std::path::PathBuf::from(root)
    };
    Some((sid, root))
}

/// The agent's most-recently-active top-level session + its repo root.
/// `None` if the agent has never run (nowhere to deliver yet).
pub(super) fn latest_session_for_agent(
    state: &Arc<ServerState>,
    agent: &str,
) -> Option<(String, std::path::PathBuf)> {
    let (session_id, repo_path) = state.manager.latest_session(agent)?;
    Some((session_id, std::path::PathBuf::from(repo_path)))
}

// The caller supplies candidates in preference order and reserves their turn
// place while checking eligibility. None means the caller creates its own.
fn select_handoff<T>(
    candidates: Vec<(String, std::path::PathBuf)>,
    mut reserve: impl FnMut(&str) -> Option<T>,
) -> Option<(String, std::path::PathBuf, T)> {
    candidates
        .into_iter()
        .find_map(|(id, root)| reserve(&id).map(|held| (id, root, held)))
}

fn handoff_eligible(state: &Arc<ServerState>, session_id: &str) -> bool {
    state
        .manager
        .global_sessions
        .get_session_meta(session_id)
        .ok()
        .flatten()
        .is_some_and(|meta| eligible_meta(&meta))
}

fn eligible_meta(meta: &crate::state_fs::sessions::SessionMeta) -> bool {
    meta.creator != "external" && meta.runtime.is_linggen()
}

async fn new_handoff_session(
    state: &Arc<ServerState>,
    skill: Option<&str>,
    agent: &str,
) -> Option<(String, std::path::PathBuf)> {
    let now = crate::util::now_ts_secs();
    let sid = format!("sess-handoff-{}", uuid::Uuid::new_v4());
    let found = match skill {
        Some(name) => state.manager.skills.get_skill(name).await,
        None => None,
    };
    let declared = crate::server::chat::members::default_members(found.as_ref());
    let root = match &found {
        Some(s) => s.working_dir(),
        None => std::path::PathBuf::from(crate::state_fs::sessions::fallback_cwd(skill)),
    };
    let meta = crate::state_fs::sessions::SessionMeta {
        id: sid.clone(),
        title: skill.unwrap_or(agent).to_string(),
        created_at: now,
        skill: skill.map(str::to_string),
        creator: "agent".to_string(),
        cwd: root.to_string_lossy().to_string(),
        title_locked: true,
        agents: declared,
        ..Default::default()
    };
    if let Err(e) = state.manager.global_sessions.add_session(&meta) {
        tracing::warn!("[agent-chat] could not create handoff session: {e}");
        return None;
    }
    Some((sid, root))
}

/// An agent_chat message kept in a restart's plan, delivered again as it
/// came if this engine outlives the restart.
fn resend_agent_chat(
    state: Arc<ServerState>,
    (from, to, message): (&str, &str, &str),
    (app, sent_in): (Option<String>, Option<String>),
) -> crate::server::admission::Resend {
    let (from, to, message) = (from.to_string(), to.to_string(), message.to_string());
    Box::new(move || {
        Box::pin(async move {
            tokio::spawn(deliver_to_chat_agent(state, from, to, message, (app, sent_in)));
        })
    })
}

#[cfg(test)]
mod native_target_tests {
    use super::{eligible_meta, is_delegated, select_handoff};
    use crate::state_fs::sessions::SessionMeta;
    use std::path::PathBuf;

    struct Candidate {
        meta: SessionMeta,
        busy: bool,
        messages: Vec<String>,
    }

    fn candidate(id: &str, creator: &str, busy: bool) -> Candidate {
        Candidate {
            meta: SessionMeta {
                id: id.into(),
                creator: creator.into(),
                ..Default::default()
            },
            busy,
            messages: Vec::new(),
        }
    }

    fn route(candidates: &mut [Candidate]) -> String {
        let choices = candidates
            .iter()
            .map(|c| (c.meta.id.clone(), PathBuf::new()))
            .collect();
        let selected = select_handoff(choices, |id| {
            let c = candidates.iter().find(|c| c.meta.id == id).unwrap();
            (eligible_meta(&c.meta) && !c.busy).then_some(())
        });
        let id = selected
            .map(|(id, _, ())| id)
            .unwrap_or_else(|| "new-own".into());
        if let Some(c) = candidates.iter_mut().find(|c| c.meta.id == id) {
            c.messages.push("handoff".into());
        }
        id
    }

    #[test]
    fn no_app_external_only_creates_own_session() {
        let mut sessions = [candidate("external", "external", false)];
        assert_eq!(route(&mut sessions), "new-own");
        assert!(sessions[0].messages.is_empty());
    }

    #[test]
    fn no_app_own_idle_session_is_chosen() {
        let mut sessions = [candidate("own", "user", false)];
        assert_eq!(route(&mut sessions), "own");
        assert_eq!(sessions[0].messages, ["handoff"]);
    }

    #[test]
    fn app_newest_external_session_is_not_reused() {
        let mut sessions = [
            candidate("newest", "external", false),
            candidate("older", "skill", false),
        ];
        assert_eq!(route(&mut sessions), "older");
        assert!(sessions[0].messages.is_empty());
    }

    #[test]
    fn external_focused_session_is_skipped() {
        let mut sessions = [
            candidate("focused", "external", false),
            candidate("latest", "user", false),
        ];
        assert_eq!(route(&mut sessions), "latest");
        assert!(sessions[0].messages.is_empty());
    }

    #[test]
    fn another_owners_busy_latest_session_is_skipped() {
        let mut sessions = [candidate("latest", "user", true)];
        assert_eq!(route(&mut sessions), "new-own");
        assert!(sessions[0].messages.is_empty());
    }

    #[test]
    fn only_external_sessions_are_delegated() {
        for creator in ["user", "agent", "skill", "mission"] {
            let meta = SessionMeta {
                creator: creator.into(),
                ..Default::default()
            };
            assert!(!is_delegated(Some(&meta)), "{creator}");
        }
        let meta = SessionMeta {
            creator: "external".into(),
            ..Default::default()
        };
        assert!(is_delegated(Some(&meta)));
        assert!(!is_delegated(None));
    }
}

#[cfg(test)]
mod kept_tests {
    /// A question kept to an app reaches her only as that the app waits:
    /// nothing of what it asks, and no way to answer it for the person.
    #[test]
    fn a_kept_question_reaches_her_without_its_words() {
        let kickoff = super::kept_ask_kickoff("zz-ledger");
        assert!(kickoff.contains("zz-ledger"));
        assert!(!kickoff.contains("answer_prompt"));
        assert!(!kickoff.contains("options"));
    }
}

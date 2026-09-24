//! What wakes her: the event bus, mission ends, finished and failed runs,
//! pending asks, and where her words are delivered.

use super::*;

pub async fn yinyue_watch_loop(state: Arc<ServerState>) {
    let mut rx = state.events_tx.subscribe();
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
            ..
        } => {
            if agent_id == YINYUE_AGENT {
                return; // she's the one asking — not a herald
            }
            if user_sees_screen(state) {
                return; // the question is on the screen they're looking at
            }
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
            let state = state.clone();
            tokio::spawn(async move {
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
                let Some(reply) = run_yinyue_turn(&state, kickoff, "event").await else {
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
                crate::server::api::yinyue::emit_speak(&state, line, Some("neutral".to_string()));
            });
        }
        // A peer agent sent a message. TO YINYUE → she receives it in her own
        // voice (herald). TO A CHAT AGENT (Ling, …) → it shows in that agent's
        // chat as a message from the sender, and that agent responds. Either way
        // the recipient's turn is tagged `agent_chat` (one-hop loop-break).
        ServerEvent::AgentChat {
            from,
            to,
            message,
            app,
        } => {
            if from == to {
                return; // no self-send (also guarded in the tool)
            }
            let state = state.clone();
            if to == YINYUE_AGENT {
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
                            );
                        } else {
                            tracing::info!("[yinyue-watch] chose silence (agent_chat)");
                        }
                    }
                });
            } else {
                tokio::spawn(async move {
                    deliver_to_chat_agent(state, from, to, message, app).await;
                });
            }
        }
        _ => {} // firehose + everything else dropped here, near-free
    }
}

/// Dispatch a notification payload to its reaction.
pub(super) fn handle_notification(state: &Arc<ServerState>, payload: NotificationPayload) {
    match payload {
        // A background mission finished — wake Yinyue to decide whether it's worth
        // a word. A real LLM reaction, because she may have something to say.
        NotificationPayload::MissionCompleted {
            mission_id,
            mission_name,
            status,
            ..
        } => {
            // Guard: never react to a Yinyue-shaped mission (belt-and-suspenders).
            if mission_id.starts_with(YINYUE_AGENT) {
                return;
            }
            // A cancel is user-initiated — they know; announcing it as a
            // finished mission is both wrong and noise (cancel != completion).
            if status == "cancelled" {
                return;
            }
            tracing::info!(
                "[yinyue-watch] mission '{mission_id}' completed ({status}); waking Yinyue"
            );
            let state = state.clone();
            tokio::spawn(async move {
                wake_for_mission(state, &mission_name, &status).await;
            });
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
                crate::server::api::yinyue::emit_speak(&state, line, Some("sad".to_string()));
            });
        }
        // A run finished cleanly. This fires on every reply, so only herald when
        // the user has stepped away — otherwise they already see it on screen.
        NotificationPayload::RunCompleted {
            agent_id,
            session_id,
        } => {
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
                let asked = session_id
                    .as_deref()
                    .and_then(|sid| last_user_line(&state, sid))
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

/// The user's most recent message in a session — the topic of the reply
/// Yinyue announces — trimmed to a short quote. Hidden prompts and tool
/// payloads are skipped. `None` when the session has nothing quotable.
pub(super) fn last_user_line(state: &Arc<ServerState>, session_id: &str) -> Option<String> {
    let history = state
        .manager
        .global_sessions
        .get_chat_history(session_id)
        .ok()?;
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

/// Wake the Yinyue agent to react to a finished background mission. She decides
/// whether it's worth surfacing — replying `SILENT` means say nothing (the
/// never-nag discipline). Anything else is spoken to her surfaces.
pub(super) async fn wake_for_mission(state: Arc<ServerState>, mission_name: &str, status: &str) {
    let task = format!(
        "You've been woken to react to a background event on the user's machine. \
         The background job \"{mission_name}\" just finished (status: {status}). \
         Decide whether it's worth telling the user. If so, reply with one or two brief \
         sentences in your voice — what happened and anything notable (you may search memory, \
         Read, or Grep for context). Your reply will be SPOKEN ALOUD, so write plain prose, \
         no markdown. If it's routine and not worth interrupting them, reply with exactly the \
         single word SILENT and nothing else. Be brief. Never nag."
    );

    let emotion =
        if status.eq_ignore_ascii_case("completed") || status.to_lowercase().contains("success") {
            "happy"
        } else {
            "neutral"
        };
    wake_herald(state, task, emotion).await;
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
    wake_and_speak(state, kickoff, emotion, "event").await
}

/// Wake her to answer what the user asked her for: the same spoken final
/// paragraph, under the contract that offers no SILENT.
pub(crate) async fn wake_asked(
    state: Arc<ServerState>,
    kickoff: String,
    emotion: &str,
) -> Option<String> {
    wake_and_speak(state, kickoff, emotion, "asked").await
}

/// Wake her, and speak her line. Returns the line she said aloud — `None`
/// when the run failed, she produced nothing, or chose silence.
pub(super) async fn wake_and_speak(
    state: Arc<ServerState>,
    kickoff: String,
    emotion: &str,
    trigger_source: &str,
) -> Option<String> {
    // `None`: the run failed or she produced nothing.
    let reply = run_yinyue_turn(&state, kickoff, trigger_source).await?;
    let Some(line) = spoken_line(&reply) else {
        tracing::info!("[yinyue-watch] Yinyue chose silence");
        return None;
    };
    tracing::info!(
        "[yinyue-watch] Yinyue heralds ({} chars, {emotion})",
        line.len()
    );
    crate::server::api::yinyue::emit_speak(&state, line.clone(), Some(emotion.to_string()));
    Some(line)
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
    app: Option<String>,
) {
    // With an explicit app target, deliver into that app's (skill's) session so
    // the recipient runs with that app's tools. Otherwise prefer the session the
    // user is viewing; else the agent's latest.
    let resolved = match app.as_deref() {
        Some(skill) => session_for_skill(&state, skill),
        None => focused_session(&state).or_else(|| latest_session_for_agent(&state, &to)),
    };
    let Some((session_id, root)) = resolved else {
        tracing::info!(
            "[agent-chat] '{from}'→'{to}'{}: no session to deliver to — dropped",
            app.as_deref()
                .map(|a| format!(" (app {a})"))
                .unwrap_or_default()
        );
        return;
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
    // No run is begun here: the turn core tracks its own; one around it was
    // a second "running" row per message, and the stop button's pick could
    // be the one the engine never checks.
    // Show the incoming message in the chat, attributed to the sender. The
    // attribution is the message's from_id — the chat surfaces render the
    // "[Yinyue]" label from that one fact, and the model gets the same label
    // via ChatRunCtx::labeled_msg. The content itself stays clean.
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
    )
    .await;

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
            guest: false,
            silence_ok: false,
        };
        crate::server::chat::run_session_turn(&ctx, &mut engine, &state.manager, None).await;
        state.manager.clear_agent_chat_session(&session_id);
    }

    tracing::info!("[agent-chat] delivered '{from}'→'{to}' in {session_id}");
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

/// Resolve the session bound to an app/skill — the latest existing one (so it
/// lands in the open app tab when there is one), else a freshly minted
/// skill-bound session so the request still runs with that app's tools.
pub(super) fn session_for_skill(
    state: &Arc<ServerState>,
    skill: &str,
) -> Option<(String, std::path::PathBuf)> {
    let home = crate::util::resolve_path(std::path::Path::new("~/.linggen"));
    if let Ok(sessions) = state.manager.global_sessions.list_sessions() {
        if let Some(meta) = sessions
            .into_iter()
            .filter(|s| s.skill.as_deref() == Some(skill))
            .max_by_key(|s| s.created_at)
        {
            let root = meta
                .cwd
                .or(meta.project)
                .map(std::path::PathBuf::from)
                .unwrap_or(home);
            return Some((meta.id, root));
        }
    }
    // None yet — mint a skill-bound session so "tell Yinyue → app" works first time.
    let now = crate::util::now_ts_secs();
    let sid = format!("sess-{skill}-{now}");
    let meta = crate::state_fs::sessions::SessionMeta {
        id: sid.clone(),
        title: skill.to_string(),
        created_at: now,
        updated_at: 0,
        skill: Some(skill.to_string()),
        creator: "agent".to_string(),
        cwd: Some(home.to_string_lossy().to_string()),
        title_locked: true,
        ..Default::default()
    };
    if let Err(e) = state.manager.global_sessions.add_session(&meta) {
        tracing::warn!("[agent-chat] could not create '{skill}' session: {e}");
        return None;
    }
    Some((sid, home))
}

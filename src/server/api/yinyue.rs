//! Yinyue-specific HTTP endpoints.
//!
//! `emit_speak` pushes a "speak" cue onto the event bus; it fans out
//! to every connected surface (pet / menubar / web overlay) over the WebRTC
//! data channel. Surfaces render the bubble + expression and fetch the audio
//! from `/api/tts` — the cue is small, the blob is pulled. See the "Adaptive
//! presentation / One event spine" section of `doc/yinyue-spec.md`.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use crate::server::{ServerEvent, ServerState};

/// Where a line of hers is said: its session (`None`: her own thread) — with
/// the daily session on, the line reaches only the stage that session's
/// person holds — and the place in it whose page shows the line: an app's
/// skill, `None` Linggen's main page.
#[derive(Clone, Copy, Debug, Default)]
pub struct SaidAt<'a> {
    pub session: Option<&'a str>,
    pub app: Option<&'a str>,
}

/// Push a speak cue to the user's surfaces. The single producer — the
/// event-reactive watch loop calls this when Yinyue reacts.
pub fn emit_speak(state: &Arc<ServerState>, text: String, emotion: Option<String>, at: SaidAt<'_>) {
    emit_speak_offering(state, text, emotion, at, None);
}

/// A line that raised a problem, with its offers under her labels — the
/// buttons the bubble shows (`problem`: its key, what answers are kept on).
pub(crate) fn emit_speak_offering(
    state: &Arc<ServerState>,
    text: String,
    emotion: Option<String>,
    at: SaidAt<'_>,
    problem: Option<(String, Vec<crate::perception::offer::Shown>)>,
) {
    // Not met yet: there is no one to speak.
    if !state
        .manager
        .agent_present(crate::engine::agent::COMPANION_AGENT_ID)
    {
        return;
    }
    let voice = speaks_aloud(state, at.app);
    let session = at.session;
    // Every line she says, from any path, restarts the quiet an app moment
    // waits for — a line an app had her say directly counts too.
    crate::server::yinyue_moments::note_spoke();
    // Err only means no surface is currently connected — nothing to hear her.
    let (problem, offers) = match problem {
        Some((key, offers)) => (Some(key), offers),
        None => (None, Vec::new()),
    };
    let _ = state.events_tx.send(ServerEvent::PetSpeak {
        text,
        emotion,
        voice,
        session_id: session.map(str::to_string),
        problem,
        offers,
    });
}

/// The one place a line of hers is decided spoken: her words are said aloud
/// only when the person cannot see them. At the page the line shows on
/// (`app`, `None` the main page), or with her voice muted, it goes out as
/// words only — the stage shows the bubble and does not talk.
fn speaks_aloud(state: &ServerState, app: Option<&str>) -> bool {
    use crate::server::resident::{user_sees, Sight};
    !state.manager.pet_muted() && !user_sees(state, Sight::Line(app))
}

/// `/mute` → turn her voice off, `/unmute` → back on; any other message is
/// not a voice command. Caught before any model runs, in every chat on this
/// machine.
pub(crate) fn voice_command(message: &str) -> Option<bool> {
    match message.trim().to_ascii_lowercase().as_str() {
        "/mute" => Some(true),
        "/unmute" => Some(false),
        _ => None,
    }
}

/// Carry out a voice command; returns the system's line for the chat.
pub(crate) async fn run_voice_command(state: &Arc<ServerState>, muted: bool) -> &'static str {
    match state.manager.set_pet_muted(muted).await {
        Ok(()) if muted => {
            "Yinyue's voice is off on this Mac. She still writes — /unmute brings it back."
        }
        Ok(()) => "Yinyue's voice is back on on this Mac.",
        Err(e) => {
            tracing::warn!("[yinyue] voice command failed: {e}");
            "Couldn't change Yinyue's voice — try again."
        }
    }
}

/// The person answered a problem her line raised on this Mac: "not now", or
/// what a tapped offer came to. Kept on the problem's one record.
#[derive(Deserialize)]
pub(crate) struct AnswerRequest {
    pub problem: String,
    /// `not_now`, `done`, `failed`, `declined`.
    pub answer: String,
    /// What the tapped offer returned — she says the outcome from it.
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    /// Her label on the offer that was tapped.
    #[serde(default)]
    pub label: Option<String>,
}

/// `POST /api/yinyue/answer` — `{problem, answer, result?, label?}`. This
/// Mac's own problems only: the other device's are answered where they live.
/// A tap's outcome (`result`) is said by her, as an answer to the person.
pub(crate) async fn answer_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<AnswerRequest>,
) -> impl IntoResponse {
    let Some(answer) = crate::perception::told::Answer::parse(&req.answer) else {
        return (StatusCode::BAD_REQUEST, "answer is not_now, done, failed or declined").into_response();
    };
    let problem = req.problem.trim();
    if problem.is_empty() || problem.len() > 32 || problem.contains('/') {
        return (StatusCode::BAD_REQUEST, "not a problem of this Mac's").into_response();
    }
    answer_problem(problem, answer, crate::util::now_ts_secs() as i64);
    if let (Some(result), Some(label)) = (req.result, req.label) {
        let result = match result {
            serde_json::Value::String(s) => s,
            v => v.to_string(),
        };
        crate::server::resident::offer_outcome::say(&state, label, req.answer, result);
    }
    StatusCode::NO_CONTENT.into_response()
}

/// Keep the answer on the problem's record.
fn answer_problem(problem: &str, answer: crate::perception::told::Answer, now: i64) {
    crate::perception::told::update(|ledger| ledger.answered(problem, answer, 1, now));
}

#[derive(Deserialize)]
pub(crate) struct EventRequest {
    pub app: String,
    pub text: String,
    #[serde(default)]
    pub big: bool,
    /// The user asked her for this (a reading): answered at once.
    #[serde(default)]
    pub asked: bool,
    /// A big moment on the screen right now: she is woken at once.
    #[serde(default)]
    pub now: bool,
    #[serde(default)]
    pub mood: Option<String>,
    /// The app's chat session: her line lands there too, as her message.
    #[serde(default)]
    pub session: Option<String>,
    /// With `session`: after her line, the session's agent answers her once.
    #[serde(default)]
    pub converse: bool,
    /// The app's own language (a BCP-47 tag: `en`, `zh-Hans`…), when it plays
    /// in another than Linggen's: her line for this moment is in it.
    #[serde(default)]
    pub lang: Option<String>,
    /// The sender's person (the channel and the LAN gate set it): with the daily session on, only the builder's moments wake her.
    #[serde(default)]
    pub user_id: Option<String>,
}

const EVENT_TEXT_MAX: usize = 300;

/// An app's language tag, cut to what Linggen writes by (`language::normalize`);
/// anything that is not a short tag is dropped, not refused.
fn event_language(tag: Option<&str>) -> Option<String> {
    let tag = tag?.trim();
    let shaped = !tag.is_empty()
        && tag.len() <= 16
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    shaped.then(|| crate::language::normalize(tag))
}

/// POST /api/yinyue/event — `{ app, text, big?, asked?, mood?, session?,
/// converse? }`. An app tells Yinyue what just happened, as a plain fact.
/// Nothing is said now: the moment waits until the user has gone quiet (or,
/// `big`, until the screen settles), and then she judges whether a word fits
/// (server/yinyue_moments.rs). A `session` has her line land in that chat.
pub(crate) async fn event_handler(
    State(state): State<Arc<ServerState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<EventRequest>,
) -> impl IntoResponse {
    let app = req.app.trim();
    let text = req.text.trim();
    if app.is_empty() || app.len() > 40 || text.is_empty() {
        return (StatusCode::BAD_REQUEST, "app and text are required").into_response();
    }
    let text: String = text.chars().take(EVENT_TEXT_MAX).collect();
    // A mood is a word for her face; anything else is dropped, not refused.
    let mood = req
        .mood
        .filter(|m| !m.is_empty() && m.len() <= 16 && m.chars().all(|c| c.is_ascii_lowercase()));
    tracing::info!(
        "[yinyue] app moment from {app} ({} chars, big={})",
        text.len(),
        req.big
    );
    let session = req
        .session
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(sid) = session.as_deref() {
        if !matches!(
            state.manager.global_sessions.get_session_meta(sid),
            Ok(Some(_))
        ) {
            return (StatusCode::BAD_REQUEST, "unknown session").into_response();
        }
    }
    // A moment is its sender's: another person's is not taken up, as one
    // from a world she is not in.
    let theirs = !crate::server::day_session::shares_builders(&state.manager, &crate::server::person::Person::from_wire(req.user_id.as_deref())).await;
    // An app whose world she hasn't entered yet tells her nothing there.
    if theirs || absent_for_moment(&state, app, session.as_deref()).await {
        return (StatusCode::CONFLICT, "absent").into_response();
    }
    // Her turns run as the owner's: a moment below admin wakes none.
    if let Some(refused) = crate::server::person::owners_only(crate::server::api::pair::caller_level(&headers)) {
        return refused;
    }
    // Asked while the pet is off: nobody will answer — say so now, rather
    // than let the page wait on a silence.
    if req.asked && !state.manager.get_config_snapshot().await.pet.enabled {
        return (StatusCode::SERVICE_UNAVAILABLE, "unanswered: pet off").into_response();
    }
    crate::server::yinyue_moments::push(crate::server::yinyue_moments::Moment {
        app: app.to_string(),
        text,
        big: req.big,
        asked: req.asked,
        now: req.now && req.big,
        mood,
        at: crate::util::now_ts_secs(),
        converse: req.converse && session.is_some(),
        session,
        lang: event_language(req.lang.as_deref()),
    });
    (StatusCode::OK, "ok").into_response()
}

/// Whether the app a moment comes from keeps her away right now — by the
/// skill of the chat it names, else the skill named as the app.
async fn absent_for_moment(state: &Arc<ServerState>, app: &str, session: Option<&str>) -> bool {
    use crate::server::chat::presence;
    let companion = crate::engine::agent::COMPANION_AGENT_ID;
    match session {
        Some(sid) => presence::absent_in_session(&state.manager, sid, companion).await,
        None => presence::absent_in_skill(&state.manager, app, companion).await,
    }
}

#[derive(Deserialize)]
pub(crate) struct ChatRequest {
    pub text: String,
    /// The caller's person (the channel and the LAN gate set it): her own
    /// thread is the builder's day, so with the daily session on another
    /// person is refused it.
    #[serde(default)]
    pub user_id: Option<String>,
}

/// POST /api/yinyue/chat — `{ text }`. The user talks to Yinyue directly (e.g.
/// clicking the desktop avatar). Her turn runs in the background and her reply
/// is spoken over the event spine; returns immediately so the UI isn't blocked.
pub(crate) async fn chat_handler(
    State(state): State<Arc<ServerState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<ChatRequest>,
) -> impl IntoResponse {
    let text = req.text.trim().to_string();
    if text.is_empty() {
        return (StatusCode::BAD_REQUEST, "empty text").into_response();
    }
    if !crate::server::day_session::shares_builders(&state.manager, &crate::server::person::Person::from_wire(req.user_id.as_deref())).await {
        return (StatusCode::FORBIDDEN, "her own thread is another person's").into_response();
    }
    // Her own thread runs with the owner's tools: below admin it is not
    // talked into (a device talks in its own chat and day).
    if let Some(refused) = crate::server::person::owners_only(crate::server::api::pair::caller_level(&headers)) {
        return refused;
    }
    // Not met yet: she is not here to be talked to.
    if !state
        .manager
        .agent_present_now(crate::engine::agent::COMPANION_AGENT_ID)
        .await
    {
        return (StatusCode::CONFLICT, "absent").into_response();
    }
    if let Some(muted) = voice_command(&text) {
        // The avatar has no chat to write a line into: the surface shows
        // the change from the voice event.
        run_voice_command(&state, muted).await;
        return (StatusCode::OK, "ok").into_response();
    }
    // A private app taken up by name is carried to its own chat, words and
    // all: nothing of it is a row of her thread.
    if let Some(app) = crate::server::day_session::private_app_named(&state.manager, &text).await {
        let companion = crate::engine::agent::COMPANION_AGENT_ID;
        crate::server::chat::say_in_app(state.clone(), &app, companion, &text).await;
        return (StatusCode::OK, "ok").into_response();
    }
    if let Some(session_id) = presenting_in_chat(&state, req.user_id.as_deref()).await {
        say_at_table(&state, session_id, &text).await;
        return (StatusCode::OK, "ok").into_response();
    }
    tracing::info!("[yinyue] chat from user ({} chars)", text.len());
    // If a worker agent is currently blocked on a prompt, frame this turn so she
    // can relay the answer with `answer_prompt` rather than just chatting — the
    // user's reply and the open prompt land in the same turn (no cross-turn recall).
    let task = frame_with_pending_prompt(&state, &text).await;
    tokio::spawn(async move {
        if let Some(reply) = crate::server::resident::run_yinyue_turn(&state, task, "user").await {
            if !reply.eq_ignore_ascii_case("silent") {
                emit_speak(&state, reply, None, SaidAt::default());
            }
        }
    });
    (StatusCode::OK, "ok").into_response()
}

/// The app chat she is presenting in for the caller `user_id` names — a
/// stage of theirs on an app page with a chat holds her — when that chat is
/// there and she is on. One conversation per app: what the user says to her
/// then is said in that chat.
async fn presenting_in_chat(state: &Arc<ServerState>, user_id: Option<&str>) -> Option<String> {
    let sid = state.yinyue_stage_session(crate::server::person::Person::from_wire(user_id))?;
    if crate::server::resident::is_own_session(&sid)
        || !state.manager.get_config_snapshot().await.pet.enabled
    {
        return None;
    }
    let meta = state
        .manager
        .global_sessions
        .get_session_meta(&sid)
        .ok()??;
    // Not there yet in that app's world: she answers on her own thread.
    let companion = crate::engine::agent::COMPANION_AGENT_ID;
    if crate::server::chat::presence::absent_in_session(&state.manager, &meta.id, companion).await {
        return None;
    }
    Some(meta.id)
}

/// Say `text` to her in the app chat, exactly as if typed there to her: the
/// user's line in the chat, her turn there as one of its members, spoken.
async fn say_at_table(state: &Arc<ServerState>, session_id: String, text: &str) {
    tracing::info!(
        "[yinyue] chat from user ({} chars) → the app chat {session_id}",
        text.len()
    );
    let companion = crate::engine::agent::COMPANION_AGENT_ID;
    crate::server::chat::say_in_session(state.clone(), &session_id, companion, text).await;
}

/// If another agent is parked on a prompt, wrap the user's message with that
/// context (question, options, `question_id`) so Yinyue can relay it with
/// `answer_prompt`. Plain message through when nothing is waiting. A
/// question kept to a private app (`turn_place::question_kept_to`) is
/// framed only as that the app waits: nothing of it, and no id to answer.
async fn frame_with_pending_prompt(state: &Arc<ServerState>, text: &str) -> String {
    let first = {
        let pending = state.pending_ask_user.lock().await;
        pending
            .iter()
            .find(|(_, p)| p.agent_id != crate::engine::agent::COMPANION_AGENT_ID)
            .map(|(qid, p)| (qid.clone(), p.agent_id.clone(), p.session_id.clone(), p.questions.first().cloned()))
    };
    let Some((qid, agent, session, question)) = first else {
        return text.to_string();
    };
    match crate::engine::turn_place::question_kept_to(&state.manager, session.as_deref()).await {
        Some(app) => frame_kept(&app, text),
        None => frame_open(&agent, &qid, question.as_ref(), text),
    }
}

/// The user's message with a question it may answer: what it asks, its
/// options and id.
fn frame_open(agent: &str, qid: &str, q0: Option<&crate::engine::tools::AskUserQuestion>, text: &str) -> String {
    let question = q0.map(|q| q.question.as_str()).unwrap_or("their input");
    let options = q0
        .map(|q| {
            q.options
                .iter()
                .map(|o| o.label.clone())
                .collect::<Vec<_>>()
                .join(" / ")
        })
        .unwrap_or_default();
    format!(
        "[The agent \"{agent}\" is blocked, waiting on an answer to: \"{question}\" \
         (options: {options}; question_id \"{qid}\"). If the user's message below is their \
         answer, relay it with `answer_prompt` — only their actual words, never your own \
         decision. Otherwise just talk with them normally.]\n\nUser says: {text}"
    )
}

/// The user's message beside a question kept to the private app `app`:
/// only that the app waits, answered in the app's own chat.
fn frame_kept(app: &str, text: &str) -> String {
    format!(
        "[The app \"{app}\" is waiting for the user's answer to a question there; what it asks is \
         kept to that app and is answered only in its own chat.]\n\nUser says: {text}"
    )
}

#[derive(Deserialize)]
pub(crate) struct PresenceBeat {
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub typing: bool,
    /// Milliseconds since the user's last input (key/pointer), measured client-side.
    #[serde(default)]
    pub idle_ms: u64,
    /// The app (skill) this surface shows, if it is one — lets an app's
    /// moments wait until that app is the one in front.
    #[serde(default)]
    pub app: Option<String>,
    /// This surface's own id, minted once per page load — the engine keeps
    /// one reading per surface. Old clients send none.
    #[serde(default)]
    pub surface: Option<String>,
    /// The caller's person (the channel and the LAN gate set it).
    #[serde(default)]
    pub user_id: Option<String>,
}

/// POST /api/presence — a throttled liveness beat from a client surface. Carries
/// only recency + focus + a typing flag (never keystroke content); feeds the
/// `sense` tool so Yinyue can tell whether the user is here, reading, or away.
/// Generic (not Yinyue-specific) but lives here as the companion is its only
/// consumer today. With the daily session on, it is the builder's presence:
/// another person's beat is taken and dropped.
pub(crate) async fn presence_handler(
    State(state): State<Arc<ServerState>>,
    Json(beat): Json<PresenceBeat>,
) -> impl IntoResponse {
    // Presence is the owner's: another person's surface, or a peer that
    // named no device, says nothing of whether the owner is here.
    let asker = crate::server::person::Person::from_wire(beat.user_id.as_deref());
    let theirs = !crate::server::day_session::shares_builders(&state.manager, &asker).await;
    if theirs || !crate::server::day_session::is_builder(&asker) {
        return (StatusCode::OK, "ok").into_response();
    }
    let short = |v: &String| !v.is_empty() && v.len() <= 64;
    state.manager.update_presence(crate::engine::agent::Beat {
        surface: beat.surface.filter(short),
        focused: beat.focused,
        typing: beat.typing,
        idle_ms: beat.idle_ms,
        app: beat.app.filter(|a| !a.is_empty() && a.len() <= 40),
    });
    (StatusCode::OK, "ok").into_response()
}

/// One place decides whether a line of hers is heard: her cue is sent from
/// this file alone, its voice set by [`speaks_aloud`], and nothing else asks
/// whether a line is seen. A second sender or a second judgement fails here
/// until it is looked at and listed.
#[cfg(test)]
mod one_voice_rule {
    fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// Each file's count of `needle`, against the files it must be in.
    fn check(needle: &str, listed: &[(&str, usize)]) {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        for dir in ["src", "crates/linggen-engine/src"] {
            rust_files(&root.join(dir), &mut files);
        }
        for file in files {
            let rel = file.strip_prefix(root).unwrap().to_string_lossy().to_string();
            let found = std::fs::read_to_string(&file).unwrap().matches(needle).count();
            let want = listed.iter().find(|(f, _)| *f == rel).map_or(0, |e| e.1);
            assert_eq!(found, want, "{rel}: `{needle}` — her voice has one rule; look at it, then list it");
        }
    }

    #[test]
    fn her_cue_is_sent_and_its_voice_decided_in_one_place() {
        // Built from parts, so this test's own text is no match.
        let send = format!("{}{}", "events_tx.send(ServerEvent::", "PetSpeak");
        check(&send, &[("src/server/api/yinyue.rs", 1)]);
        let voice = format!("{}{}", "speaks_", "aloud(");
        check(&voice, &[("src/server/api/yinyue.rs", 2)]);
        let seen = format!("{}{}", "Sight::", "Line");
        check(&seen, &[("src/server/api/yinyue.rs", 1), ("src/server/resident/triggers.rs", 1)]);
    }
}

#[cfg(test)]
mod voice_tests {
    use super::voice_command;

    #[test]
    fn mute_and_unmute_are_commands_and_nothing_else_is() {
        assert_eq!(voice_command("/mute"), Some(true));
        assert_eq!(voice_command("  /UNMUTE \n"), Some(false));
        for other in [
            "mute",
            "/mute please",
            "/muted",
            "/unmute me",
            "please /mute",
            "",
        ] {
            assert_eq!(voice_command(other), None, "{other:?}");
        }
    }
}

#[cfg(test)]
mod event_language_tests {
    use super::event_language;

    #[test]
    fn an_event_names_its_language_as_a_short_tag_or_not_at_all() {
        assert_eq!(event_language(Some("en")).as_deref(), Some("en"));
        assert_eq!(event_language(Some("zh")).as_deref(), Some("zh-Hans"));
        assert_eq!(event_language(Some(" en-CA ")).as_deref(), Some("en"));
        assert_eq!(event_language(None), None);
        assert_eq!(event_language(Some("")), None);
        assert_eq!(event_language(Some("Reply in French please")), None);
        assert_eq!(event_language(Some("x".repeat(40).as_str())), None);
    }
}

//! App moments — what happens in an app, told to Yinyue, and a word from her
//! only when the room has gone quiet.
//!
//! An app (a game, a player, anything on a page) posts what just happened as a
//! plain fact: `POST /api/yinyue/event { app, text, big?, asked?, mood?,
//! session?, converse? }`. Nothing is
//! said then. The moments wait here until the user has been at the screen and
//! still for a while — or, for a `big` one (a loss, a hard win), until the
//! screen has settled — and only then is she woken, once, with everything that
//! piled up since she last spoke. She may say one line, or `SILENT`.
//!
//! The engine names no app: it carries the app's own words to her. Why a
//! queue and a gate rather than a wake per event: an LLM asked to respond
//! always responds, so a companion who is told every event talks over the
//! game. Silence is decided here, in code (the lesson Intra's lunch scene
//! taught — six NPCs, six monologues a turn).
//!
//! A moment that names the app's chat `session` has her line land there too,
//! as a message from her — in the chat, and in the context of the session's
//! agent (Ling) — without waking her. A `converse` moment then gives her ONE
//! hidden kickoff to answer her in a line, or SILENT; never back to her, and
//! at most once per [`CONVERSE_GAP_SECS`] per session.
//!
//! Her moment turn itself reaches no one: it cannot `agent_chat` (the tool is
//! withheld — a message from her would land in the app's chat and wake its
//! agent on words meant for her). The `converse` kickoff is the only exchange.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::state::ServerState;

/// How often the gate is checked. Cheap: a lock, a few numbers, no model.
const TICK_SECS: u64 = 5;
/// The user at the screen with no input this long reads as "quiet for a while".
const QUIET_SECS: u64 = 90;
/// No new moment this long — an app still animating keeps posting.
const SETTLE_SECS: u64 = 20;
/// At least this long since she last said anything, from any path.
const COOLDOWN_SECS: u64 = 600;
/// A `big` moment skips the quiet wait but not the screen settling…
const BIG_SETTLE_SECS: u64 = 6;
/// …nor a shorter cooldown.
const BIG_COOLDOWN_SECS: u64 = 180;
/// A moment older than this is no longer worth a word.
const STALE_SECS: u64 = 15 * 60;
/// The most moments kept; the oldest go first.
const MAX_MOMENTS: usize = 24;
/// A presence beat older than this means no surface is showing the app.
const BEAT_FRESH_SECS: u64 = 60;
/// At least this long between two conversational exchanges in one session.
const CONVERSE_GAP_SECS: u64 = 120;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Moment {
    pub app: String,
    pub text: String,
    pub big: bool,
    /// The user asked HER for this — a reading, a word. It is answered at
    /// once: no quiet to wait for, no cooldown, and silence is not an answer.
    pub asked: bool,
    /// A big moment the app is showing right now (a seal, a burst): woken at
    /// once, like `asked`, so her line meets the picture — but worded as a
    /// moment, not a question, and she may still stay silent.
    pub now: bool,
    pub mood: Option<String>,
    pub at: u64,
    /// The app's chat session: her line lands there as a message from her.
    pub session: Option<String>,
    /// After her line lands, the session's agent answers her once.
    pub converse: bool,
}

static MOMENTS: Mutex<VecDeque<Moment>> = Mutex::new(VecDeque::new());
/// When she last said anything aloud — stamped by `emit_speak`, so a line an
/// app had her say directly counts too.
static LAST_SPOKE_AT: AtomicU64 = AtomicU64::new(0);
/// When the gate last woke her, whether or not she spoke.
static LAST_WAKE_AT: AtomicU64 = AtomicU64::new(0);

pub(crate) fn note_spoke() {
    LAST_SPOKE_AT.store(crate::util::now_ts_secs(), Ordering::Relaxed);
}

/// The asked moments a wake is answering right now. While it runs, no other
/// wake starts — what arrives meanwhile waits and goes in one wake after it.
static IN_FLIGHT: Mutex<Vec<Moment>> = Mutex::new(Vec::new());

/// The same ask again — a double tap — while the first is queued or being
/// answered: one answer covers both.
fn is_repeat_ask(moment: &Moment, queued: &VecDeque<Moment>, in_flight: &[Moment]) -> bool {
    let same = |m: &Moment| m.asked && m.app == moment.app && m.text == moment.text;
    moment.asked && (queued.iter().any(same) || in_flight.iter().any(same))
}

pub(crate) fn push(moment: Moment) {
    let mut q = MOMENTS.lock().unwrap_or_else(|e| e.into_inner());
    let in_flight = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
    if is_repeat_ask(&moment, &q, &in_flight) {
        tracing::info!(
            "[yinyue-moments] the same ask from {} again; one answer covers it",
            moment.app
        );
        return;
    }
    drop(in_flight);
    q.push_back(moment);
    while q.len() > MAX_MOMENTS {
        q.pop_front();
    }
}

/// What the gate reads of the room — the presence beat, reduced.
#[derive(Clone, Debug)]
pub(crate) struct Room {
    /// A live, focused surface: the user can see the app.
    pub at_screen: bool,
    pub typing: bool,
    /// Seconds since their last key or pointer input.
    pub idle: u64,
    /// The app the focused surface shows, when its beat names one.
    pub app: Option<String>,
}

impl Room {
    /// The user can see where these moments happened: at the screen, and
    /// the app in front is one of theirs (or the beat names none).
    fn sees(&self, moments: &[Moment]) -> bool {
        self.at_screen
            && self
                .app
                .as_deref()
                .is_none_or(|app| moments.iter().any(|m| m.app == app))
    }
}

/// Whether to wake her now. Pure, so the thresholds are tested, not trusted.
pub(crate) fn ready(now: u64, room: &Room, moments: &[Moment], last_voice: u64) -> bool {
    let Some(newest) = moments.iter().map(|m| m.at).max() else {
        return false;
    };
    // Asked for, it is answered now (they just tapped; they are here).
    if moments.iter().any(|m| m.asked) {
        return true;
    }
    // Shown right now: her word belongs with the picture, not after it.
    if moments.iter().any(|m| m.now) && room.sees(moments) {
        return true;
    }
    // Never to an empty room or another app's screen, never over their typing.
    if !room.sees(moments) || room.typing {
        return false;
    }
    let since_voice = now.saturating_sub(last_voice);
    let settled = now.saturating_sub(newest);
    if moments.iter().any(|m| m.big)
        && settled >= BIG_SETTLE_SECS
        && since_voice >= BIG_COOLDOWN_SECS
    {
        return true;
    }
    room.idle >= QUIET_SECS && settled >= SETTLE_SECS && since_voice >= COOLDOWN_SECS
}

/// The moments as the lines she reads, oldest first.
fn fact_lines(moments: &[Moment]) -> String {
    moments
        .iter()
        .map(|m| format!("- {}", m.text))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The apps the moments came from, in order, each once.
fn app_names(moments: &[Moment]) -> String {
    let mut apps: Vec<&str> = Vec::new();
    for m in moments {
        if !apps.contains(&m.app.as_str()) {
            apps.push(&m.app);
        }
    }
    apps.join(", ")
}

/// Every moment kickoff ends here: her line is the whole of her answer. The
/// turn cannot message another agent anyway (`resident::wake_for_moment`
/// withholds `agent_chat`); this keeps her from reaching for it (2026-09-24:
/// an idle moment's kickoff was relayed to Ling as a task).
const YOURS_ALONE: &str = "This is yours alone: your line is all you give, and it reaches \
     the user and the app's chat by itself. Never pass this on or message another agent about it.";

/// The kickoff she is woken with. The app's facts, oldest first, and what she
/// is there for — never an instruction to speak.
pub(crate) fn kickoff(moments: &[Moment]) -> String {
    format!(
        "While the user was in {}, this happened (oldest first):\n{}\n\n\
         They are at the screen and have been quiet for a while. You are beside them in \
         this — not a narrator. If a word from you fits, say ONE short line in your own voice, \
         in the language these notes are written in: comfort after a loss or a wound, gladness \
         at something hard-won, a little courage before what's ahead, or just being there. \
         Never repeat what is already on their screen, never tell them what to do next, never \
         mention these notes or that you were told. If nothing is worth a word, SILENT. {}",
        app_names(moments),
        fact_lines(moments),
        YOURS_ALONE
    )
}

/// The kickoff when the user asked her for something: answer it, in her
/// voice. Worded for the answer contract (`resident::wake_asked`): the
/// answer is her final paragraph, the part spoken aloud.
pub(crate) fn asked_kickoff(moments: &[Moment]) -> String {
    format!(
        "The user asked you, in {}, and here is what you have to go on:\n{}\n\n\
         Answer them now, in your own voice, in the language of these notes — one short \
         paragraph of two or three sentences, plain prose, spoken aloud. Read it for them the \
         way you would: what it means, and one honest word for their day. Do not recite the \
         notes, do not mention that you were told. They are waiting for this answer. {}",
        app_names(moments),
        fact_lines(moments),
        YOURS_ALONE
    )
}

/// What one wake takes, and what stays queued. When the user asked for
/// something, only the asked moments are answered now; the rest wait for
/// the quiet as if nothing had been asked. Otherwise the wake takes all.
fn take_for_wake(moments: Vec<Moment>) -> (Vec<Moment>, Vec<Moment>) {
    let (asked, waiting): (Vec<Moment>, Vec<Moment>) = moments.into_iter().partition(|m| m.asked);
    if asked.is_empty() {
        (waiting, Vec::new())
    } else {
        (asked, waiting)
    }
}

/// The mood to wear: the newest moment that named one.
fn mood_of(moments: &[Moment]) -> String {
    moments
        .iter()
        .rev()
        .find_map(|m| m.mood.clone())
        .unwrap_or_else(|| "neutral".to_string())
}

fn room_now(state: &Arc<ServerState>, now: u64) -> Room {
    let p = state.manager.presence_snapshot();
    let fresh = p.updated_at != 0 && now.saturating_sub(p.updated_at) <= BEAT_FRESH_SECS;
    Room {
        at_screen: fresh && p.focused,
        typing: p.typing,
        idle: now.saturating_sub(p.last_input_at),
        app: p.app,
    }
}

/// Asked moments nobody will answer — the pet is off. The page that asked
/// hears so (`device_topic` yinyue/unanswered) instead of waiting on silence.
pub(crate) fn tell_unanswered(state: &Arc<ServerState>, moments: &[Moment]) {
    for m in moments.iter().filter(|m| m.asked) {
        let _ = state.events_tx.send(super::ServerEvent::DeviceTopic {
            topic: "yinyue".to_string(),
            op: "unanswered".to_string(),
            payload: serde_json::json!({ "app": m.app, "text": m.text, "reason": "pet-off" }),
            from_device: None,
        });
    }
}

/// Where her line lands: each chat session the moments named, once, in
/// order, and whether that chat asked for one exchange. Nothing when she
/// said nothing (SILENT) — silence leaves no trace in a chat.
fn landings(moments: &[Moment], spoke: bool) -> Vec<(&str, bool)> {
    let mut out: Vec<(&str, bool)> = Vec::new();
    if !spoke {
        return out;
    }
    for m in moments {
        let Some(sid) = m.session.as_deref() else {
            continue;
        };
        match out.iter_mut().find(|(s, _)| *s == sid) {
            Some(landing) => landing.1 |= m.converse,
            None => out.push((sid, m.converse)),
        }
    }
    out
}

/// The app chats the moments name, each once, in order, with its app.
fn chats_named(moments: &[Moment]) -> Vec<(&str, &str)> {
    let mut out: Vec<(&str, &str)> = Vec::new();
    for m in moments {
        let Some(sid) = m.session.as_deref() else {
            continue;
        };
        if !out.iter().any(|(_, s)| *s == sid) {
            out.push((m.app.as_str(), sid));
        }
    }
    out
}

/// One app chat's dialogue, as she reads it beside the moment.
fn aside_for(app: &str, transcript: &str) -> String {
    format!(
        "The chat in {app} so far, oldest first — everything the user sees there, you \
         included:\n{transcript}"
    )
}

/// What she reads beside the moments: the visible dialogue of each app chat
/// they name (`chat::table`). One conversation per app — the user, the
/// app's agent and her — so her line fits where it lands. Read for the turn
/// only, never kept on her thread. `None` when no chat is named or said.
async fn aside(state: &Arc<ServerState>, moments: &[Moment]) -> Option<String> {
    let root = crate::util::resolve_path(std::path::Path::new("~/.linggen"));
    let mut parts = Vec::new();
    for (app, sid) in chats_named(moments) {
        let rows = crate::server::chat::table::read(&state.manager, &root, sid).await;
        if !rows.is_empty() {
            parts.push(aside_for(
                app,
                &crate::server::chat::table::as_transcript(&rows),
            ));
        }
    }
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// When each session last had a conversational exchange.
static LAST_CONVERSE: Mutex<Vec<(String, u64)>> = Mutex::new(Vec::new());

/// Whether a session may have another exchange now. Pure.
fn converse_due(last: Option<u64>, now: u64) -> bool {
    last.is_none_or(|t| now.saturating_sub(t) >= CONVERSE_GAP_SECS)
}

/// Claim the session's exchange slot: true (and stamped) when it is due.
fn claim_converse(session_id: &str, now: u64) -> bool {
    let mut last = LAST_CONVERSE.lock().unwrap_or_else(|e| e.into_inner());
    let at = last.iter().position(|(s, _)| s == session_id);
    if !converse_due(at.map(|i| last[i].1), now) {
        return false;
    }
    match at {
        Some(i) => last[i].1 = now,
        None => last.push((session_id.to_string(), now)),
    }
    true
}

/// The agent who answers in a session: the one whose own replies are there,
/// newest first — never the companion.
fn answering_agent(rows: &[crate::state_fs::sessions::ChatMsg]) -> Option<String> {
    rows.iter()
        .rev()
        .find(|r| r.from_id == r.agent_id && r.agent_id != crate::engine::agent::COMPANION_AGENT_ID)
        .map(|r| r.agent_id.clone())
}

/// The hidden kickoff the session's agent gets after her line. Her words are
/// already in his context (the chat's newest line from her).
pub(crate) fn converse_kickoff() -> String {
    "Yinyue just spoke in this chat — her line is the newest one from her, above. \
     If a reply from you fits, answer her in ONE line, in-world and in your role here, in \
     the chat's language. Don't ask her anything back and don't start a new scene; this is \
     one exchange. If nothing fits, reply exactly SILENT."
        .to_string()
}

/// Her spoken line, landed in each app chat the moments named: a message from
/// her (the session's agent reads it on his next turn), then — for a
/// `converse` moment, when the session is due — one kickoff for that agent.
async fn land_in_chats(state: &Arc<ServerState>, moments: &[Moment], line: Option<&str>) {
    let companion = crate::engine::agent::COMPANION_AGENT_ID;
    for (sid, wants_reply) in landings(moments, line.is_some()) {
        let Some(line) = line else { break };
        let Ok(Some(_)) = state.manager.global_sessions.get_session_meta(sid) else {
            continue; // the session is gone
        };
        // Not there (any more) in that app's world: her line lands nowhere
        // she isn't — the gate is read again as it lands, not only when the
        // moment was posted.
        if crate::server::chat::presence::absent_in_session(&state.manager, sid, companion).await {
            continue;
        }
        crate::server::chat::helpers::persist_and_emit_to_store(
            &state.manager.global_sessions,
            &state.events_tx,
            companion,
            companion,
            "user",
            line,
            Some(sid),
            false,
        )
        .await;
        crate::server::chat::side_lines::note(
            sid,
            companion,
            format!("[{}]: {line}", crate::server::chat::sender_label(companion)),
        );
        if !wants_reply || !claim_converse(sid, crate::util::now_ts_secs()) {
            continue;
        }
        let rows = state
            .manager
            .global_sessions
            .get_chat_history(sid)
            .unwrap_or_default();
        let Some(agent) = answering_agent(&rows) else {
            continue; // nobody has answered in this chat yet
        };
        tracing::info!("[yinyue-moments] one exchange: {agent} answers her in {sid}");
        crate::server::chat::kickoff_in_session(state.clone(), sid, &agent, &converse_kickoff())
            .await;
    }
}

pub async fn yinyue_moment_loop(state: Arc<ServerState>) {
    tracing::info!("[yinyue-moments] started");
    loop {
        tokio::time::sleep(Duration::from_secs(TICK_SECS)).await;
        let now = crate::util::now_ts_secs();
        if !IN_FLIGHT
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
        {
            continue; // one wake at a time; what arrives meanwhile goes next, together
        }
        let taken = {
            let mut q = MOMENTS.lock().unwrap_or_else(|e| e.into_inner());
            q.retain(|m| now.saturating_sub(m.at) < STALE_SECS);
            if q.is_empty() {
                continue;
            }
            let last_voice = LAST_SPOKE_AT
                .load(Ordering::Relaxed)
                .max(LAST_WAKE_AT.load(Ordering::Relaxed));
            let moments: Vec<Moment> = q.iter().cloned().collect();
            if !ready(now, &room_now(&state, now), &moments, last_voice) {
                continue;
            }
            let (taken, stay) = take_for_wake(moments);
            q.clear();
            q.extend(stay);
            taken
        };
        if !state.manager.get_config_snapshot().await.pet.enabled {
            tell_unanswered(&state, &taken);
            continue; // pet off: the moments are dropped, nobody to say them
        }
        LAST_WAKE_AT.store(now, Ordering::Relaxed);
        let asked = taken[0].asked;
        tracing::info!(
            "[yinyue-moments] waking her with {} {}moment(s) from {}",
            taken.len(),
            if asked { "asked " } else { "" },
            app_names(&taken)
        );
        let emotion = mood_of(&taken);
        *IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner()) = taken.clone();
        let state = state.clone();
        tokio::spawn(async move {
            let words = if asked {
                asked_kickoff(&taken)
            } else {
                kickoff(&taken)
            };
            let aside = aside(&state, &taken).await;
            let table = chats_named(&taken).first().map(|(_, sid)| sid.to_string());
            let line = super::resident::wake_for_moment(
                state.clone(),
                (words, aside, table),
                &emotion,
                asked,
            )
            .await;
            IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner()).clear();
            land_in_chats(&state, &taken, line.as_deref()).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(at: u64, big: bool) -> Moment {
        Moment {
            app: "game".into(),
            text: "雷神放出雷霆".into(),
            big,
            asked: false,
            now: false,
            mood: None,
            at,
            session: None,
            converse: false,
        }
    }
    const HERE: Room = Room {
        at_screen: true,
        typing: false,
        idle: 100,
        app: None,
    };

    #[test]
    fn a_now_moment_wakes_her_at_once_while_it_is_seen() {
        let shown = Moment {
            now: true,
            ..m(1000, true)
        };
        // No settle, no cooldown: the seal is on the screen this second.
        assert!(ready(1000, &HERE, std::slice::from_ref(&shown), 999));
        // Not to an empty room.
        let away = Room {
            at_screen: false,
            ..HERE
        };
        assert!(!ready(1000, &away, std::slice::from_ref(&shown), 999));
        // A plain big moment still waits for the screen to settle.
        assert!(!ready(1000, &HERE, &[m(1000, true)], 0));
    }

    #[test]
    fn nothing_to_say_without_moments() {
        assert!(!ready(10_000, &HERE, &[], 0));
    }

    #[test]
    fn a_plain_moment_waits_for_quiet_settling_and_the_cooldown() {
        let now = 10_000;
        assert!(ready(now, &HERE, &[m(now - 30, false)], 0));
        // still typing, or away, or the tab hidden: never
        assert!(!ready(
            now,
            &Room {
                typing: true,
                ..HERE
            },
            &[m(now - 30, false)],
            0
        ));
        assert!(!ready(
            now,
            &Room {
                at_screen: false,
                ..HERE
            },
            &[m(now - 30, false)],
            0
        ));
        // not quiet long enough
        assert!(!ready(
            now,
            &Room { idle: 40, ..HERE },
            &[m(now - 30, false)],
            0
        ));
        // the app still posting
        assert!(!ready(now, &HERE, &[m(now - 5, false)], 0));
        // she spoke five minutes ago
        assert!(!ready(now, &HERE, &[m(now - 30, false)], now - 300));
        // quiet a long while at the screen is still quiet — no upper bound
        assert!(ready(
            now,
            &Room {
                idle: 1_000,
                ..HERE
            },
            &[m(now - 30, false)],
            0
        ));
    }

    #[test]
    fn a_big_moment_skips_the_quiet_wait_but_not_the_screen_or_cooldown() {
        let now = 10_000;
        let busy = Room { idle: 8, ..HERE };
        assert!(ready(now, &busy, &[m(now - 10, true)], 0));
        assert!(
            !ready(now, &busy, &[m(now - 2, true)], 0),
            "the screen settles first"
        );
        assert!(
            !ready(now, &busy, &[m(now - 10, true)], now - 60),
            "a shorter cooldown still holds"
        );
        assert!(!ready(
            now,
            &Room {
                typing: true,
                ..busy
            },
            &[m(now - 10, true)],
            0
        ));
    }

    #[test]
    fn the_kickoff_carries_the_facts_in_order_and_never_orders_her_to_speak() {
        let k = kickoff(&[
            Moment {
                app: "game".into(),
                text: "打赢了夔".into(),
                big: false,
                asked: false,
                now: false,
                mood: None,
                at: 1,
                session: None,
                converse: false,
            },
            Moment {
                app: "game".into(),
                text: "气血只剩 6".into(),
                big: true,
                asked: false,
                now: false,
                mood: Some("sad".into()),
                at: 2,
                session: None,
                converse: false,
            },
        ]);
        assert!(k.starts_with("While the user was in game,"));
        assert!(k.find("打赢了夔").unwrap() < k.find("气血只剩 6").unwrap());
        assert!(k.contains("SILENT"));
        assert_eq!(mood_of(&[m(1, false)]), "neutral");
    }

    /// Both kickoffs say her line is the whole answer — never something to
    /// hand on (2026-09-24: the idle kickoff went to Ling as a task).
    #[test]
    fn every_moment_kickoff_says_her_line_is_hers_alone() {
        let idle = m(1, false);
        let asked = Moment {
            asked: true,
            now: false,
            ..m(1, false)
        };
        for k in [kickoff(&[idle]), asked_kickoff(&[asked])] {
            assert!(k.contains(YOURS_ALONE), "{k}");
        }
        assert!(YOURS_ALONE.contains("message another agent"));
    }

    #[test]
    fn asked_is_answered_at_once_and_never_with_silence() {
        let now = 10_000;
        let asked = Moment {
            asked: true,
            now: false,
            ..m(now, false)
        };
        // she spoke a minute ago, the user is mid-typing, the moment is a second old: still now
        assert!(ready(
            now,
            &Room {
                typing: true,
                idle: 0,
                at_screen: false,
                app: None,
            },
            &[asked.clone()],
            now - 60
        ));
        let k = asked_kickoff(&[asked]);
        assert!(k.contains("雷神放出雷霆"));
        assert!(
            !k.contains("SILENT"),
            "the answer contract offers no silence"
        );
    }

    #[test]
    fn a_moment_waits_until_its_own_app_is_in_front() {
        let now = 10_000;
        let other = Room {
            app: Some("player".into()),
            ..HERE
        };
        assert!(
            !ready(now, &other, &[m(now - 30, false)], 0),
            "another app is in front"
        );
        let own = Room {
            app: Some("game".into()),
            ..HERE
        };
        assert!(ready(now, &own, &[m(now - 30, false)], 0));
    }

    #[test]
    fn a_double_tap_is_one_ask() {
        let ask = Moment {
            asked: true,
            now: false,
            ..m(1, false)
        };
        let queued: VecDeque<Moment> = [ask.clone()].into_iter().collect();
        assert!(is_repeat_ask(&ask, &queued, &[]), "queued");
        assert!(
            is_repeat_ask(&ask, &VecDeque::new(), &[ask.clone()]),
            "being answered"
        );
        assert!(!is_repeat_ask(&ask, &VecDeque::new(), &[]));
        let plain = m(1, false);
        assert!(
            !is_repeat_ask(&plain, &[plain.clone()].into_iter().collect(), &[]),
            "only asks coalesce"
        );
    }

    #[test]
    fn an_ask_takes_only_the_asked_and_names_their_app() {
        let quiet = m(1, false);
        let asked = Moment {
            app: "cfo".into(),
            text: "NVDA 21% of the portfolio".into(),
            asked: true,
            now: false,
            ..m(2, false)
        };
        let (taken, stay) = take_for_wake(vec![quiet.clone(), asked.clone()]);
        assert_eq!(taken, vec![asked]);
        assert_eq!(stay, vec![quiet.clone()], "the unasked wait for the quiet");
        let k = asked_kickoff(&taken);
        assert!(k.starts_with("The user asked you, in cfo,"));
        assert!(!k.contains("雷神"));

        let (taken, stay) = take_for_wake(vec![quiet.clone()]);
        assert_eq!(taken, vec![quiet]);
        assert!(stay.is_empty());
    }

    fn in_chat(session: Option<&str>, converse: bool) -> Moment {
        Moment {
            session: session.map(str::to_string),
            converse,
            ..m(1, true)
        }
    }

    #[test]
    fn her_line_lands_in_each_named_chat_once_and_silence_lands_nowhere() {
        let moments = [
            in_chat(Some("s1"), false),
            in_chat(None, false),
            in_chat(Some("s1"), true),
            in_chat(Some("s2"), false),
        ];
        assert_eq!(landings(&moments, true), vec![("s1", true), ("s2", false)]);
        assert!(
            landings(&moments, false).is_empty(),
            "SILENT appends nothing"
        );
        assert!(
            landings(&[in_chat(None, true)], true).is_empty(),
            "no session, no chat"
        );
    }

    #[test]
    fn a_moment_turn_reads_each_named_chat_once_with_its_app() {
        let mut a = in_chat(Some("s1"), false);
        a.app = "lingjing".into();
        let mut b = in_chat(Some("s1"), true);
        b.app = "lingjing".into();
        let none = in_chat(None, false);
        let mut c = in_chat(Some("s2"), false);
        c.app = "dj".into();
        assert_eq!(
            chats_named(&[a, none.clone(), b, c]),
            [("lingjing", "s1"), ("dj", "s2")]
        );
        assert!(chats_named(&[none]).is_empty(), "no chat, nothing to read");
        let text = aside_for("lingjing", "[User]: 去临淄\n[Ling]: 到了。");
        assert!(text.contains("lingjing") && text.ends_with("[Ling]: 到了。"));
    }

    #[test]
    fn one_exchange_per_session_per_gap() {
        assert!(converse_due(None, 1_000));
        assert!(!converse_due(Some(1_000), 1_000 + CONVERSE_GAP_SECS - 1));
        assert!(converse_due(Some(1_000), 1_000 + CONVERSE_GAP_SECS));
        assert!(claim_converse("s-conv-1", 5_000));
        assert!(
            !claim_converse("s-conv-1", 5_030),
            "30 s later: still resting"
        );
        assert!(claim_converse("s-conv-2", 5_030), "another chat is its own");
        assert!(claim_converse("s-conv-1", 5_000 + CONVERSE_GAP_SECS));
    }

    #[test]
    fn the_answering_agent_is_whoever_answers_there_never_her() {
        let row = |agent: &str, from: &str| crate::state_fs::sessions::ChatMsg {
            agent_id: agent.into(),
            from_id: from.into(),
            to_id: "user".into(),
            content: "…".into(),
            timestamp: 0,
            is_observation: false,
        };
        let rows = [
            row("ling", "user"),
            row("ling", "ling"),
            row("yinyue", "user"),
            row("yinyue", "yinyue"),
        ];
        assert_eq!(answering_agent(&rows).as_deref(), Some("ling"));
        assert_eq!(answering_agent(&rows[2..]), None);
        let k = converse_kickoff();
        assert!(k.contains("SILENT") && k.contains("ONE line"));
    }
}

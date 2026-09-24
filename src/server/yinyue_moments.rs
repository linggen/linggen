//! App moments — what happens in an app, told to Yinyue, and a word from her
//! only when the room has gone quiet.
//!
//! An app (a game, a player, anything on a page) posts what just happened as a
//! plain fact: `POST /api/yinyue/event { app, text, big?, mood? }`. Nothing is
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

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Moment {
    pub app: String,
    pub text: String,
    pub big: bool,
    /// The user asked HER for this — a reading, a word. It is answered at
    /// once: no quiet to wait for, no cooldown, and silence is not an answer.
    pub asked: bool,
    pub mood: Option<String>,
    pub at: u64,
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
         mention these notes or that you were told. If nothing is worth a word, SILENT.",
        app_names(moments),
        fact_lines(moments)
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
         notes, do not mention that you were told. They are waiting for this answer.",
        app_names(moments),
        fact_lines(moments)
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
            if asked {
                super::resident::wake_asked(state, asked_kickoff(&taken), &emotion).await;
            } else {
                super::resident::wake_herald(state, kickoff(&taken), &emotion).await;
            }
            IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner()).clear();
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
            mood: None,
            at,
        }
    }
    const HERE: Room = Room {
        at_screen: true,
        typing: false,
        idle: 100,
        app: None,
    };

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
                mood: None,
                at: 1,
            },
            Moment {
                app: "game".into(),
                text: "气血只剩 6".into(),
                big: true,
                asked: false,
                mood: Some("sad".into()),
                at: 2,
            },
        ]);
        assert!(k.starts_with("While the user was in game,"));
        assert!(k.find("打赢了夔").unwrap() < k.find("气血只剩 6").unwrap());
        assert!(k.contains("SILENT"));
        assert_eq!(mood_of(&[m(1, false)]), "neutral");
    }

    #[test]
    fn asked_is_answered_at_once_and_never_with_silence() {
        let now = 10_000;
        let asked = Moment {
            asked: true,
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
}

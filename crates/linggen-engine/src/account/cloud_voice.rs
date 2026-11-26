//! Linggen Cloud's voice — the two uses a subscription pays for besides chat:
//! the live lesson's key (`POST /api/live/token`) and reading aloud
//! (`POST /api/tts`), both on linggen.dev with the account key.
//!
//! "Linggen Cloud" means WHO PAYS (Linggen's keys), not the path the traffic
//! takes: a live lesson's sound goes straight from the device to the provider
//! with a short-lived key this module asks the site for; read-aloud text goes
//! through the site. This is the one module that talks to linggen.dev for
//! either; `live.rs` and the TTS source ask here and name no URL.
//!
//! Nothing here logs a token, a key or a text: a refusal is one of [`Denied`]'s
//! fixed words, read from the site's own fixed error codes.

use super::{http, resolve_token, site_url};
use crate::util::LockExt;
use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The read-aloud model and voice Linggen Cloud offers (its card's line,
/// `providers_view::cloud_uses`), and the site writes them last on its side (`cloudTts.ts`). Yinyue's voice.
pub const SPEECH_MODEL: &str = "qwen3-tts-flash";
pub const SPEECH_VOICE: &str = "Serena";

/// Why Linggen Cloud's voice was not given. A fixed word each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denied {
    /// No account key on this machine, or the site refused it.
    SignedOut,
    /// Signed in, but no subscription (a trial does not cover voice).
    NotSubscribed,
    /// Today's read-aloud, or today's live keys, are used up.
    DailyCap,
    /// This month's included live lessons are used up.
    LessonsUsed,
    /// The site or the provider behind it is failing.
    Down,
    /// The site could not be reached in time.
    Unreachable,
    /// The site answered something this engine cannot use.
    BadAnswer,
}

impl Denied {
    /// How the site's answer reads: its status and, for 429, its fixed code.
    fn from_site(status: u16, code: &str) -> Denied {
        match (status, code) {
            (401, _) => Denied::SignedOut,
            (402, _) => Denied::NotSubscribed,
            (429, "lessons_used") => Denied::LessonsUsed,
            (429, _) => Denied::DailyCap,
            (400..=499, _) => Denied::BadAnswer,
            _ => Denied::Down,
        }
    }

    /// How long reading aloud stops asking the site after this, so a line of
    /// many clauses is not many refused calls: the local voice speaks meanwhile.
    fn cool_off(self) -> Duration {
        match self {
            Denied::DailyCap => Duration::from_secs(3600),
            Denied::SignedOut | Denied::NotSubscribed => Duration::from_secs(600),
            Denied::LessonsUsed | Denied::Down | Denied::Unreachable | Denied::BadAnswer => {
                Duration::from_secs(120)
            }
        }
    }
}

/// What the site gave for one live lesson: the socket address WITH the short
/// key in it, the key, when it ends, and the lesson's length cap.
pub struct Grant {
    pub ws_url: String,
    pub token: String,
    pub expires: String,
    pub max_minutes: Option<u32>,
    pub lessons_left: Option<u32>,
}

static SPEECH_COOL_OFF: Mutex<Option<Instant>> = Mutex::new(None);

/// Sign-out drops the cool-off: the next account starts clean.
pub(crate) fn forget() {
    *SPEECH_COOL_OFF.lock_ok() = None;
}

fn cooling_off() -> bool {
    SPEECH_COOL_OFF
        .lock_ok()
        .is_some_and(|until| Instant::now() < until)
}

/// Whether Linggen Cloud's read-aloud can be asked for now — a fact read from
/// disk and memory, no network: signed in, entitled as far as this machine
/// knows (`account::subscribed_known`), and not cooling off after a refusal.
/// `Source::readiness` and the status route use it, so they start nothing.
pub fn speech_usable() -> bool {
    super::subscribed_known() && !cooling_off()
}

/// Whether a live key can be asked for now: signed in and entitled, as far as
/// this machine knows. Its allowances are the site's to say when asked.
pub fn live_usable() -> bool {
    super::subscribed_known()
}

fn token() -> Result<String, Denied> {
    resolve_token().map(|(t, _)| t).ok_or(Denied::SignedOut)
}

/// The site's fixed error code in a refusal's body, or "".
async fn code_of(resp: reqwest::Response) -> String {
    let body: Value = resp.json().await.unwrap_or_default();
    body.get("error")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// A live lesson's key for `provider` (the engine's own provider id, which the
/// site names the same way). The answer is the engine's own grant shape.
pub async fn live_grant(provider: &str) -> Result<Grant, Denied> {
    let key = token()?;
    let resp = http()
        .post(format!("{}/api/live/token", site_url()))
        .bearer_auth(&key)
        .json(&json!({ "provider": provider }))
        .send()
        .await
        .map_err(|_| Denied::Unreachable)?;
    let status = resp.status().as_u16();
    if status == 401 {
        super::note_token_rejected(&key);
    }
    if !(200..300).contains(&status) {
        return Err(Denied::from_site(status, &code_of(resp).await));
    }
    let answer: Value = resp.json().await.map_err(|_| Denied::BadAnswer)?;
    read_grant(&answer, provider).ok_or(Denied::BadAnswer)
}

/// The grant in the site's answer, when it is for `provider` and its socket is a secure one.
fn read_grant(answer: &Value, provider: &str) -> Option<Grant> {
    let text = |name: &str| {
        answer
            .get(name)
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
    };
    let count = |name: &str| {
        answer
            .get(name)
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
    };
    if text("provider")? != provider {
        return None;
    }
    let ws_url = text("ws_url").filter(|u| u.starts_with("wss://"))?;
    Some(Grant {
        ws_url,
        token: text("token")?,
        expires: text("expires")?,
        max_minutes: count("max_minutes"),
        lessons_left: count("lessons_left"),
    })
}

/// `text` read aloud in Yinyue's Cloud voice: one WAV clip. One line of at most
/// 600 characters (the site's limit); the caller cuts a long line into clauses.
/// After a refusal this returns it at once for a while (`cool_off`) instead of
/// asking again.
pub async fn speak(text: &str) -> Result<Vec<u8>, Denied> {
    let result = speak_once(text).await;
    if let Err(why) = result {
        *SPEECH_COOL_OFF.lock_ok() = Some(Instant::now() + why.cool_off());
    }
    result
}

async fn speak_once(text: &str) -> Result<Vec<u8>, Denied> {
    let key = token()?;
    let resp = http()
        .post(format!("{}/api/tts", site_url()))
        .bearer_auth(&key)
        .timeout(Duration::from_secs(20))
        .json(&json!({ "text": text }))
        .send()
        .await
        .map_err(|_| Denied::Unreachable)?;
    let status = resp.status().as_u16();
    if status == 401 {
        super::note_token_rejected(&key);
    }
    if !(200..300).contains(&status) {
        return Err(Denied::from_site(status, &code_of(resp).await));
    }
    let wav = resp.bytes().await.map_err(|_| Denied::Unreachable)?;
    // The clips are joined by their decoded samples later; a body that is not a WAV is no clip.
    if wav.len() < 12 || &wav[..4] != b"RIFF" {
        return Err(Denied::BadAnswer);
    }
    Ok(wav.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sites_answer_reads_as_one_fixed_word() {
        assert_eq!(Denied::from_site(401, ""), Denied::SignedOut);
        assert_eq!(
            Denied::from_site(402, "not_subscribed"),
            Denied::NotSubscribed
        );
        assert_eq!(Denied::from_site(429, "lessons_used"), Denied::LessonsUsed);
        assert_eq!(Denied::from_site(429, "daily_cap"), Denied::DailyCap);
        assert_eq!(Denied::from_site(429, "anything"), Denied::DailyCap);
        assert_eq!(Denied::from_site(400, "too_long"), Denied::BadAnswer);
        assert_eq!(Denied::from_site(502, "key_refused"), Denied::Down);
        assert_eq!(Denied::from_site(503, "not_configured"), Denied::Down);
    }

    #[test]
    fn a_grant_is_for_the_provider_asked_and_a_secure_socket() {
        let good = json!({
            "provider": "alibaba", "model": "m", "token": "st-1", "expires": "2027-01-15T08:00:00Z",
            "ws_url": "wss://h/p?api_key=st-1", "max_minutes": 20, "lessons_left": 11,
        });
        let g = read_grant(&good, "alibaba").unwrap();
        assert_eq!((g.max_minutes, g.lessons_left), (Some(20), Some(11)));
        assert!(
            read_grant(&good, "google").is_none(),
            "another provider's grant is not this one's"
        );
        let mut plain = good.clone();
        plain["ws_url"] = json!("ws://h/p");
        assert!(
            read_grant(&plain, "alibaba").is_none(),
            "a socket must be secure"
        );
        let mut bare = good.clone();
        bare["token"] = json!("");
        assert!(read_grant(&bare, "alibaba").is_none());
        let mut open_ended = good;
        open_ended["lessons_left"] = Value::Null;
        assert_eq!(
            read_grant(&open_ended, "alibaba").unwrap().lessons_left,
            None,
            "the developer's lessons are not counted"
        );
    }

    /// A refusal stops the engine asking for a while, and sign-out lifts it.
    #[test]
    fn a_refusal_cools_off_reading_aloud_until_sign_out() {
        let _guard = super::super::cloud::TEST_LOCK.lock_ok();
        forget();
        assert!(!cooling_off());
        *SPEECH_COOL_OFF.lock_ok() = Some(Instant::now() + Denied::DailyCap.cool_off());
        assert!(cooling_off());
        forget();
        assert!(!cooling_off());
    }

    /// Lock: this module logs nothing, so no token, key or text can reach a log.
    #[test]
    fn nothing_here_logs() {
        let code = include_str!("cloud_voice.rs");
        let production = &code[..code.find("#[cfg(test)]").unwrap()];
        for word in [
            "tracing::",
            "info!",
            "warn!",
            "error!",
            "debug!",
            "println!",
        ] {
            assert!(
                !production.contains(word),
                "cloud_voice.rs may not use `{word}`"
            );
        }
    }
}

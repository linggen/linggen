//! Live voice: the short-lived token an app's page needs to open a provider's
//! live socket itself (`POST /api/live/token`, `GET /api/live`; the handlers
//! are `server/api/live.rs`).
//!
//! The page never holds the provider's key. The engine, which holds the keys,
//! asks the provider for a token that is good for ONE live session, fixed to
//! one model and to audio output, and hands the page only that.
//!
//! Providers are a declared list ([`PROVIDERS`]): adding one is one entry
//! here. Everything a provider decides — where its token endpoint is, which
//! model, how its request and answer look, the name its key is stored under —
//! is in its entry; the code below it names no provider. The engine names no
//! app: any page may ask.
//!
//! Secrets: the key and the token travel only as [`Secret`], whose `Debug`
//! prints nothing of it, and no line of this module logs, formats into an
//! error, or returns a provider's body. A failure is one of [`Refusal`]'s
//! fixed codes. The key is read in [`find_key`] and sent in [`mint`], and
//! nowhere else.

use crate::config::ModelConfig;
use crate::credentials::{self, Credentials};
use chrono::{DateTime, Duration as Span, Utc};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A key or a token. `Debug` and `Display` say nothing of the value, so a
/// stray `{:?}` or `{}` cannot write it to a log or an error; the one way to
/// the text is [`Secret::expose`], called where the value is sent or handed
/// to the page.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(text: impl Into<String>) -> Self {
        Secret(text.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// What a provider's token answer comes to.
struct Answer {
    token: Secret,
    /// When the provider says the token ends; none when its answer holds only
    /// the token (then it ends when this engine asked: `token_minutes`).
    expires: Option<String>,
}

/// One live provider: everything that differs between providers.
pub struct Provider {
    /// The name a page asks for (`{provider: "google"}`).
    pub id: &'static str,
    /// The model every session of this provider is fixed to.
    pub model: &'static str,
    /// The provider's HTTP origin.
    http_base: &'static str,
    /// Where a token is asked for, under the origin.
    token_path: &'static str,
    /// The header that carries the key on that request.
    key_header: &'static str,
    /// The live socket's address, without the token.
    socket_url: &'static str,
    /// The provider name the key is stored under (Settings → Models) and the
    /// host that name's account is at: a key stored for that name at another
    /// host (a gateway) is not this provider's.
    key_provider: &'static str,
    key_host: &'static str,
    /// The scratch engines' knob: when this environment variable is set it is
    /// the key for this route, and for this route only (`find_key`).
    key_env: &'static str,
    /// How long a token lives, as this engine asks for it. Also the expiry
    /// told to the page when the provider's answer names none.
    token_minutes: i64,
    /// The token request's body, fixing the model and audio output.
    body: fn(&Provider, DateTime<Utc>) -> Value,
    /// The token and its expiry out of the provider's answer.
    read: fn(&Value) -> Option<Answer>,
    /// The address a page opens, from the token.
    socket_with: fn(&Provider, &Secret) -> String,
}

/// The live providers this engine knows. An open set: a new one is one entry.
pub const PROVIDERS: &[Provider] = &[GOOGLE];

const GOOGLE: Provider = Provider {
    id: "google",
    model: "gemini-3.8-live",
    http_base: "https://generativelanguage.googleapis.com",
    token_path: "/v1beta/auth_tokens",
    key_header: "x-goog-api-key",
    socket_url: "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContentConstrained",
    key_provider: "gemini",
    key_host: "generativelanguage.googleapis.com",
    key_env: "LINGGEN_LIVE_GOOGLE_KEY",
    token_minutes: 30,
    body: google_body,
    read: google_answer,
    socket_with: google_socket,
};

/// A token good for one session, alive 30 minutes, to be used within one
/// minute, fixed to the model and to audio output. The field is
/// `bidiGenerateContentSetup`: the service refuses the SDK's
/// `liveConnectConstraints` (400, measured 2026-11-22).
fn google_body(p: &Provider, now: DateTime<Utc>) -> Value {
    json!({
        "uses": 1,
        "expireTime": stamp(now + Span::minutes(p.token_minutes)),
        "newSessionExpireTime": stamp(now + Span::minutes(1)),
        "bidiGenerateContentSetup": {
            "model": format!("models/{}", p.model),
            "generationConfig": { "responseModalities": ["AUDIO"] },
        },
    })
}

/// A time as a provider reads it: UTC, to the second.
fn stamp(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// The token is the answer's `name`. The real answer holds nothing else
/// (seen 2026-11-23); an `expireTime` is read when one is there.
fn google_answer(v: &Value) -> Option<Answer> {
    let token = v.get("name")?.as_str().filter(|t| !t.is_empty())?;
    let expires = v.get("expireTime").and_then(Value::as_str).map(str::to_string);
    Some(Answer {
        token: Secret::new(token),
        expires,
    })
}

fn google_socket(p: &Provider, token: &Secret) -> String {
    format!("{}?access_token={}", p.socket_url, token.expose())
}

/// The provider a page named.
pub fn provider_named(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

/// Why a token was not made — a fixed word for each, so nothing of the
/// provider's body, the key or an error text reaches the page or the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The page named a provider this engine does not list.
    UnknownProvider,
    /// No key is stored for the provider.
    NoKey,
    /// Too many tokens asked for in a minute.
    SlowDown,
    /// The provider did not accept the key (401, 403).
    KeyRefused,
    /// The provider refused the request (400, other 4xx).
    RequestRefused,
    /// The provider is down or failing (5xx, or its own rate limit).
    ProviderDown,
    /// The provider could not be reached in time.
    Unreachable,
    /// The provider answered something that holds no token.
    BadAnswer,
}

impl Refusal {
    /// The word the page reads.
    pub fn code(self) -> &'static str {
        match self {
            Refusal::UnknownProvider => "unknown_provider",
            Refusal::NoKey => "no_key",
            Refusal::SlowDown => "slow_down",
            Refusal::KeyRefused => "key_refused",
            Refusal::RequestRefused => "request_refused",
            Refusal::ProviderDown => "provider_down",
            Refusal::Unreachable => "unreachable",
            Refusal::BadAnswer => "bad_answer",
        }
    }

    /// The HTTP status the route answers with.
    pub fn status(self) -> u16 {
        match self {
            Refusal::UnknownProvider => 400,
            Refusal::NoKey => 409,
            Refusal::SlowDown => 429,
            Refusal::KeyRefused
            | Refusal::RequestRefused
            | Refusal::ProviderDown
            | Refusal::Unreachable
            | Refusal::BadAnswer => 502,
        }
    }
}

/// Where the provider's HTTP origin is. Always the declared one — except in
/// a test build (the `test-support` feature, which a shipped `ling` is not
/// built with), where `LINGGEN_LIVE_<ID>_BASE` in the engine's own
/// environment points it at a loopback stand-in. A page or a request cannot
/// reach it: an environment variable is set when the process starts.
pub fn http_base(p: &Provider) -> String {
    #[cfg(any(test, feature = "test-support"))]
    if let Ok(base) = std::env::var(format!("LINGGEN_LIVE_{}_BASE", p.id.to_uppercase())) {
        if !base.trim().is_empty() {
            return base.trim().trim_end_matches('/').to_string();
        }
    }
    p.http_base.to_string()
}

/// The key for `p`, or none: the scratch-engine knob first (`key_env`, set
/// for this route alone), else the key stored for the provider — found by
/// provider and host, with no model call ([`credentials::resolve_provider_key`]),
/// from `~/.linggen/credentials.json` read fresh as web search reads its key.
/// The only place a key is read.
pub fn find_key(p: &Provider, configured: &[ModelConfig]) -> Option<Secret> {
    let stored = || {
        let creds = Credentials::load(&credentials::credentials_file());
        credentials::resolve_provider_key(p.key_provider, p.key_host, configured, &creds)
    };
    pick_key(std::env::var(p.key_env).ok(), stored)
}

/// The knob wins when set and not blank; otherwise what is stored.
fn pick_key(knob: Option<String>, stored: impl FnOnce() -> Option<String>) -> Option<Secret> {
    knob.map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .or_else(stored)
        .map(Secret::new)
}

/// One new live session's grant.
#[derive(Debug)]
pub struct Minted {
    pub provider: &'static str,
    pub model: &'static str,
    pub ws_url: Secret,
    pub token: Secret,
    pub expires: String,
}

/// Ask the provider at `base` for a token for one session. The one place the
/// key is sent and the one outside request of this feature; the answer is
/// read for the token and its expiry and nothing else is kept or returned.
pub async fn mint(
    p: &'static Provider,
    base: &str,
    key: &Secret,
    now: DateTime<Utc>,
) -> Result<Minted, Refusal> {
    let mut client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none());
    if base.starts_with("http://") {
        // Only a test stand-in is plain HTTP; it is reached directly.
        client = client.no_proxy();
    }
    let client = client.build().map_err(|_| Refusal::Unreachable)?;
    let sent = client
        .post(format!("{base}{}", p.token_path))
        .header(p.key_header, key.expose())
        .json(&(p.body)(p, now))
        .send()
        .await;
    let response = sent.map_err(|_| Refusal::Unreachable)?;
    let status = response.status().as_u16();
    match status {
        200..=299 => {}
        401 | 403 => return Err(Refusal::KeyRefused),
        429 | 500..=599 => return Err(Refusal::ProviderDown),
        _ => return Err(Refusal::RequestRefused),
    }
    let answer: Value = response.json().await.map_err(|_| Refusal::BadAnswer)?;
    let Answer { token, expires } = (p.read)(&answer).ok_or(Refusal::BadAnswer)?;
    let expires = expires.unwrap_or_else(|| stamp(now + Span::minutes(p.token_minutes)));
    Ok(Minted {
        provider: p.id,
        model: p.model,
        ws_url: Secret::new((p.socket_with)(p, &token)),
        token,
        expires,
    })
}

/// A small in-memory limit on tokens made: at most `max` in `window`, so a
/// page in a loop cannot mint hundreds.
pub struct TokenRate {
    hits: Mutex<VecDeque<Instant>>,
    max: usize,
    window: Duration,
}

impl TokenRate {
    pub fn new(max: usize, window: Duration) -> Self {
        Self {
            hits: Mutex::new(VecDeque::new()),
            max,
            window,
        }
    }

    /// Whether one more token may be asked for at `now`; if so, it is counted.
    pub fn allow(&self, now: Instant) -> bool {
        let mut hits = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        while hits
            .front()
            .is_some_and(|t| now.duration_since(*t) >= self.window)
        {
            hits.pop_front();
        }
        if hits.len() >= self.max {
            return false;
        }
        hits.push_back(now);
        true
    }
}

impl Default for TokenRate {
    /// Five a minute: a lesson opens one token, a reconnect another.
    fn default() -> Self {
        Self::new(5, Duration::from_secs(60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn google() -> &'static Provider {
        provider_named("google").unwrap()
    }

    #[test]
    fn the_request_fixes_the_model_audio_and_one_use() {
        let now = DateTime::parse_from_rfc3339("2026-11-22T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let body = (google().body)(google(), now);
        assert_eq!(body["uses"], 1);
        assert_eq!(body["expireTime"], "2026-11-22T12:30:00Z");
        assert_eq!(body["newSessionExpireTime"], "2026-11-22T12:01:00Z");
        assert_eq!(
            body["bidiGenerateContentSetup"]["model"],
            "models/gemini-3.8-live"
        );
        assert_eq!(
            body["bidiGenerateContentSetup"]["generationConfig"]["responseModalities"],
            json!(["AUDIO"])
        );
        assert!(
            body.get("liveConnectConstraints").is_none(),
            "the service refuses the SDK's name"
        );
    }

    #[test]
    fn a_secret_says_nothing_of_itself() {
        let key = Secret::new("AIza-very-secret");
        assert!(!format!("{key:?}").contains("secret"));
        let minted = Minted {
            provider: "google",
            model: "m",
            ws_url: Secret::new("wss://h/p?access_token=tok-secret"),
            token: Secret::new("tok-secret"),
            expires: "x".into(),
        };
        assert!(!format!("{minted:?}").contains("secret"));
    }

    #[test]
    fn the_knob_wins_when_set_and_blank_is_unset() {
        let stored = || Some("stored".to_string());
        assert_eq!(
            pick_key(Some(" knob ".into()), stored).unwrap().expose(),
            "knob"
        );
        assert_eq!(
            pick_key(Some("  ".into()), stored).unwrap().expose(),
            "stored"
        );
        assert_eq!(pick_key(None, stored).unwrap().expose(), "stored");
        assert!(pick_key(None, || None).is_none());
    }

    #[test]
    fn the_answer_needs_a_token_and_may_name_its_expiry() {
        let ok = json!({"name": "auth_tokens/abc", "expireTime": "2026-11-22T12:30:00Z"});
        let read = (google().read)(&ok).unwrap();
        assert_eq!(read.token.expose(), "auth_tokens/abc");
        assert_eq!(read.expires.as_deref(), Some("2026-11-22T12:30:00Z"));
        // The real service answers with the token alone (seen 2026-11-23).
        let bare = (google().read)(&json!({"name": "auth_tokens/abc"})).unwrap();
        assert_eq!(bare.token.expose(), "auth_tokens/abc");
        assert_eq!(bare.expires, None);
        assert!((google().read)(&json!({"expireTime": "x"})).is_none());
        assert!((google().read)(&json!({"name": "", "expireTime": "x"})).is_none());
    }

    /// Lock: the key is read in one function and sent in one; a secret is
    /// opened only where it is sent or handed over (the key's header, the
    /// socket address); this module logs nothing and formats no error text.
    #[test]
    fn a_secret_is_read_and_sent_in_one_place_each() {
        let code = include_str!("live.rs");
        let production = &code[..code.find("#[cfg(test)]").unwrap()];
        assert_eq!(
            production.matches("std::env::var(p.key_env)").count(),
            1,
            "the key is read in find_key alone"
        );
        assert_eq!(
            production.matches("credentials_file()").count(),
            1,
            "the stored key is read in find_key alone"
        );
        assert_eq!(
            production.matches(".expose()").count(),
            2,
            "the key's header and the socket address open a secret"
        );
        assert_eq!(
            production.matches("key.expose()").count(),
            1,
            "mint sends the key"
        );
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
                "live.rs may not use `{word}`: a secret could reach a log or an error"
            );
        }
    }

    #[test]
    fn the_rate_limit_counts_a_window() {
        let rate = TokenRate::new(3, Duration::from_secs(60));
        let t0 = Instant::now();
        assert!(rate.allow(t0) && rate.allow(t0) && rate.allow(t0));
        assert!(!rate.allow(t0 + Duration::from_secs(59)));
        assert!(rate.allow(t0 + Duration::from_secs(61)));
    }

    #[test]
    fn every_refusal_has_its_own_word_and_a_status() {
        let all = [
            Refusal::UnknownProvider,
            Refusal::NoKey,
            Refusal::SlowDown,
            Refusal::KeyRefused,
            Refusal::RequestRefused,
            Refusal::ProviderDown,
            Refusal::Unreachable,
            Refusal::BadAnswer,
        ];
        let words: std::collections::HashSet<_> = all.iter().map(|r| r.code()).collect();
        assert_eq!(words.len(), all.len());
        assert!(all
            .iter()
            .all(|r| matches!(r.status(), 400 | 409 | 429 | 502)));
    }
}

//! Live voice: the short-lived token an app's page needs to open a provider's
//! live socket itself (`POST /api/live/token`, `GET /api/live`; the handlers
//! are `server/api/live.rs`).
//!
//! Who pays is a second axis, apart from which service: a lesson on a service
//! is paid by the person's own key if there is one, else by Linggen Cloud
//! (linggen.dev gives a short key with Linggen's own key, for a subscriber —
//! `account::cloud_voice`). [`find_source`] decides it in one place; the page
//! names only the service.
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
    /// The header that carries the key on that request, and what comes
    /// before the key in its value (`Bearer ` for a service that takes the
    /// key as a bearer credential; empty when the header holds the key alone).
    key_header: &'static str,
    key_prefix: &'static str,
    /// The live socket's address, without the token.
    socket_url: &'static str,
    /// The provider name the key is stored under (Settings → Models) and the
    /// host that name's account is at: a key stored for that name at another
    /// host (a gateway) is not this provider's. `None` for a service whose
    /// key is stored as an OpenAI-compatible model at its host, under a name
    /// the owner chose and this code cannot know: any name at that host.
    key_provider: Option<&'static str>,
    key_host: &'static str,
    /// The scratch engines' knob: when this environment variable is set it is
    /// the key for this route, and for this route only (`find_key`).
    key_env: &'static str,
    /// Whether linggen.dev gives this service's lesson keys to subscribers
    /// (Linggen Cloud pays): only a service the site holds a key for.
    via_cloud: bool,
    /// How long a token lives, as this engine asks for it. Also the expiry
    /// told to the page when the provider's answer names none.
    token_minutes: i64,
    /// The token request's body: the session the page asked for (its
    /// instruction and the like — only what this entry lets a page ask), with
    /// the model and audio output fixed by the engine. `Value::Null` sends no
    /// body at all: a service whose page writes its own session on the
    /// socket takes nothing of the page's setup, and the setup is dropped.
    body: fn(&Provider, DateTime<Utc>, &Value) -> Value,
    /// The token and its expiry out of the provider's answer.
    read: fn(&Value) -> Option<Answer>,
    /// The address a page opens, from the token.
    socket_with: fn(&Provider, &Secret) -> String,
}

impl Provider {
    /// Whether Linggen Cloud (the subscription) can give this service's lesson keys.
    pub fn via_cloud(&self) -> bool {
        self.via_cloud
    }

    /// The host this provider's account is at: its line sits on the preset
    /// at this host (`providers_view`).
    pub fn key_host(&self) -> &'static str {
        self.key_host
    }
}

/// The live providers this engine knows. An open set: a new one is one entry.
///
/// **The order is the order of preference**: `GET /api/live` lists them in
/// this order and a page takes the first one that is ready. Flipping the
/// preference is one edit here.
pub const PROVIDERS: &[Provider] = &[ALIBABA, GOOGLE];

const GOOGLE: Provider = Provider {
    id: "google",
    model: "gemini-3.8-live",
    http_base: "https://generativelanguage.googleapis.com",
    token_path: "/v1beta/auth_tokens",
    key_header: "x-goog-api-key",
    key_prefix: "",
    socket_url: "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContentConstrained",
    key_provider: Some("gemini"),
    key_host: "generativelanguage.googleapis.com",
    key_env: "LINGGEN_LIVE_GOOGLE_KEY",
    via_cloud: false,
    token_minutes: 30,
    body: google_body,
    read: google_answer,
    socket_with: google_socket,
};

/// Qwen's live voice. The short key is asked with a bearer header, no body,
/// and a lifetime in the query (the service gives 60 s whatever is asked);
/// the page opens the socket with the short key in the query and writes its
/// own session there, so nothing of the page's setup is sent anywhere.
const ALIBABA: Provider = Provider {
    id: "alibaba",
    model: "qwen-audio-3.1-realtime-flash",
    http_base: "https://ap-southeast-1.maas.qwencloudapi.com",
    token_path: "/api/v1/tokens?expire_in_seconds=60",
    key_header: "Authorization",
    key_prefix: "Bearer ",
    socket_url: "wss://ap-southeast-1.maas.qwencloudapi.com/api-ws/v1/realtime",
    key_provider: None,
    key_host: "ap-southeast-1.maas.qwencloudapi.com",
    key_env: "LINGGEN_LIVE_ALIBABA_KEY",
    via_cloud: true,
    token_minutes: 1,
    body: no_body,
    read: alibaba_answer,
    socket_with: alibaba_socket,
};

/// What a page may ask of a Qwen session: nothing. The page's setup is
/// checked by the route (an object, the size cap) and dropped here.
fn no_body(_: &Provider, _: DateTime<Utc>, _asked: &Value) -> Value {
    Value::Null
}

/// The token is the answer's `token`, its end the answer's `expires_at`
/// (unix seconds). A refusal's body (`{"code": …}`) holds no token.
fn alibaba_answer(v: &Value) -> Option<Answer> {
    let token = v.get("token")?.as_str().filter(|t| !t.is_empty())?;
    let expires = v
        .get("expires_at")
        .and_then(Value::as_i64)
        .and_then(|secs| DateTime::<Utc>::from_timestamp(secs, 0))
        .map(stamp);
    Some(Answer {
        token: Secret::new(token),
        expires,
    })
}

/// The short key rides in the query: a browser cannot set a header on a
/// socket.
fn alibaba_socket(p: &Provider, token: &Secret) -> String {
    format!("{}?model={}&api_key={}", p.socket_url, p.model, token.expose())
}

/// What a page may ask of a session. Anything else in its setup is dropped
/// here, so a token never carries tools, another model or another output: a
/// live voice gets words, not tools.
const GOOGLE_PAGE_MAY_ASK: &[&str] = &[
    "systemInstruction",
    "inputAudioTranscription",
    "outputAudioTranscription",
    "sessionResumption",
    "contextWindowCompression",
];

/// A token good for one session, alive `token_minutes`, to be used within
/// one minute. The token CARRIES the session's setup: the service uses the
/// token's setup and ignores what the socket sends afterwards (measured
/// 2026-11-23 — a token fixed to the model alone dropped the page's
/// instruction). So the page's setup goes in here, cut down to
/// [`GOOGLE_PAGE_MAY_ASK`], with the model and audio output written by the
/// engine last. The field is `bidiGenerateContentSetup`: the service refuses
/// the SDK's `liveConnectConstraints` (400, measured 2026-11-22).
fn google_body(p: &Provider, now: DateTime<Utc>, asked: &Value) -> Value {
    let mut setup = serde_json::Map::new();
    for name in GOOGLE_PAGE_MAY_ASK {
        if let Some(value) = asked.get(*name) {
            setup.insert((*name).to_string(), value.clone());
        }
    }
    setup.insert("model".into(), json!(format!("models/{}", p.model)));
    setup.insert(
        "generationConfig".into(),
        json!({ "responseModalities": ["AUDIO"] }),
    );
    json!({
        "uses": 1,
        "expireTime": stamp(now + Span::minutes(p.token_minutes)),
        "newSessionExpireTime": stamp(now + Span::minutes(1)),
        "bidiGenerateContentSetup": setup,
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

/// Whether `model` at `host` is a live-voice model this engine declares
/// ([`PROVIDERS`]): the one fact `provider::kinds::kind_of` asks to tell a
/// live-voice row from a chat one.
pub fn declares(host: &str, model: &str) -> bool {
    at_model(host, model).is_some()
}

pub(crate) fn at_model(host: &str, model: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| {
        p.key_host.eq_ignore_ascii_case(host.trim()) && p.model.eq_ignore_ascii_case(model.trim())
    })
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
    /// The page's setup is not an object, or is larger than a setup is.
    BadSetup,
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
    /// Linggen Cloud's lessons are for subscribers.
    NotSubscribed,
    /// This month's included lessons are used up.
    LessonsUsed,
    /// Today's live keys are used up.
    DailyCap,
}

impl Refusal {
    /// The word the page reads.
    pub fn code(self) -> &'static str {
        match self {
            Refusal::UnknownProvider => "unknown_provider",
            Refusal::BadSetup => "bad_setup",
            Refusal::NoKey => "no_key",
            Refusal::SlowDown => "slow_down",
            Refusal::KeyRefused => "key_refused",
            Refusal::RequestRefused => "request_refused",
            Refusal::ProviderDown => "provider_down",
            Refusal::Unreachable => "unreachable",
            Refusal::BadAnswer => "bad_answer",
            Refusal::NotSubscribed => "not_subscribed",
            Refusal::LessonsUsed => "lessons_used",
            Refusal::DailyCap => "daily_cap",
        }
    }

    /// The HTTP status the route answers with.
    pub fn status(self) -> u16 {
        match self {
            Refusal::UnknownProvider | Refusal::BadSetup => 400,
            Refusal::NoKey => 409,
            Refusal::NotSubscribed => 402,
            Refusal::SlowDown | Refusal::LessonsUsed | Refusal::DailyCap => 429,
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
/// provider and host (host alone when the entry names no provider), with no
/// model call ([`credentials::resolve_provider_key`], [`credentials::resolve_host_key`]),
/// from `~/.linggen/credentials.json` read fresh as web search reads its key.
/// The only place a key is read.
pub fn find_key(p: &Provider, configured: &[ModelConfig]) -> Option<Secret> {
    provider_key(p, || {
        let creds = Credentials::load(&credentials::credentials_file());
        stored_key(p, configured, &creds)
    })
}

fn provider_key(p: &Provider, stored: impl FnOnce() -> Option<String>) -> Option<Secret> {
    pick_key(std::env::var(p.key_env).ok(), stored)
}

pub(crate) fn stored_key(p: &Provider, configured: &[ModelConfig], creds: &Credentials) -> Option<String> {
    match p.key_provider {
        Some(name) => credentials::resolve_provider_key(name, p.key_host, configured, creds),
        None => credentials::resolve_host_key(p.key_host, configured, creds),
    }
}

/// Same key sources as the live route, using the providers view's snapshot.
pub(crate) fn key_ready(p: &Provider, configured: &[ModelConfig], creds: &Credentials) -> bool {
    provider_key(p, || stored_key(p, configured, creds)).is_some()
}

/// Who pays for a lesson. A closed set: the one `match` that reads it is
/// [`find_source`], which is also where the order of preference is decided.
pub enum Source {
    /// The person's own key for the service: it costs Linggen nothing.
    OwnKey(Secret),
    /// Linggen Cloud: linggen.dev gives a short key for the lesson with ITS key,
    /// and the subscription pays (`account::cloud_voice`).
    LinggenCloud,
}

impl Source {
    /// The word `GET /api/live` tells a page.
    pub fn as_str(&self) -> &'static str {
        match self {
            Source::OwnKey(_) => "own_key",
            Source::LinggenCloud => "linggen_cloud",
        }
    }
}

/// Who pays for a lesson on `p` now — THE order of preference: the person's
/// own key first, then Linggen Cloud for a subscriber; none when neither.
/// The page names a service (`alibaba`), never who pays: this decides it.
pub fn find_source(p: &Provider, configured: &[ModelConfig]) -> Option<Source> {
    find_key(p, configured).map(Source::OwnKey).or_else(|| {
        (p.via_cloud && crate::account::cloud_voice::live_usable()).then_some(Source::LinggenCloud)
    })
}

/// Move the preferred service to the front, keeping the declared fallback order.
pub fn in_order(pref: Option<&str>) -> impl Iterator<Item = &'static Provider> {
    let preferred = pref.and_then(provider_named);
    preferred.into_iter().chain(PROVIDERS.iter().filter(move |p| Some(p.id) != preferred.map(|p| p.id)))
}

/// The one provider a page is offered: the first declared. Explicit token requests
/// can still name a development provider.
pub fn offered_to_pages() -> Option<&'static Provider> {
    PROVIDERS.first()
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
    /// A lesson's length cap in minutes, when Linggen Cloud names one (the page ends the lesson).
    pub max_minutes: Option<u32>,
    /// Lessons left this month, when Linggen Cloud counts them for this account.
    pub lessons_left: Option<u32>,
}

/// A grant for one new session from whoever pays for it.
pub async fn grant(
    p: &'static Provider,
    source: &Source,
    now: DateTime<Utc>,
    asked: &Value,
) -> Result<Minted, Refusal> {
    match source {
        Source::OwnKey(key) => mint(p, &http_base(p), key, now, asked).await,
        Source::LinggenCloud => mint_via_cloud(p).await,
    }
}

/// Linggen Cloud's grant: the site mints the provider's short key with its own
/// key and tells the engine the socket; the page's setup is written on the
/// socket by the page itself, so nothing of it is sent.
async fn mint_via_cloud(p: &'static Provider) -> Result<Minted, Refusal> {
    use crate::account::cloud_voice::{live_grant, Denied};
    let got = live_grant(p.id).await.map_err(|why| match why {
        Denied::SignedOut => Refusal::NoKey,
        Denied::NotSubscribed => Refusal::NotSubscribed,
        Denied::LessonsUsed => Refusal::LessonsUsed,
        Denied::DailyCap => Refusal::DailyCap,
        Denied::Down => Refusal::ProviderDown,
        Denied::Unreachable => Refusal::Unreachable,
        Denied::BadAnswer => Refusal::BadAnswer,
    })?;
    Ok(Minted {
        provider: p.id,
        model: p.model,
        ws_url: Secret::new(got.ws_url),
        token: Secret::new(got.token),
        expires: got.expires,
        max_minutes: got.max_minutes,
        lessons_left: got.lessons_left,
    })
}

/// Ask the provider at `base` for a token for one session. The one place the
/// key is sent and the one outside request of this feature; the answer is
/// read for the token and its expiry and nothing else is kept or returned.
pub async fn mint(
    p: &'static Provider,
    base: &str,
    key: &Secret,
    now: DateTime<Utc>,
    asked: &Value,
) -> Result<Minted, Refusal> {
    let mut client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none());
    if base.starts_with("http://") {
        // Only a test stand-in is plain HTTP; it is reached directly.
        client = client.no_proxy();
    }
    let client = client.build().map_err(|_| Refusal::Unreachable)?;
    let mut request = client
        .post(format!("{base}{}", p.token_path))
        .header(p.key_header, format!("{}{}", p.key_prefix, key.expose()));
    let body = (p.body)(p, now, asked);
    if !body.is_null() {
        request = request.json(&body);
    }
    let sent = request.send().await;
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
        max_minutes: None,
        lessons_left: None,
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

    #[test]
    fn a_live_preference_moves_only_its_provider() {
        let ids = |pref| in_order(pref).map(|p| p.id).collect::<Vec<_>>();
        assert_eq!(ids(None), ["alibaba", "google"]);
        assert_eq!(ids(Some("google")), ["google", "alibaba"]);
        assert_eq!(ids(Some("unknown")), ids(None));
    }

    fn google() -> &'static Provider {
        provider_named("google").unwrap()
    }

    #[test]
    fn the_request_fixes_the_model_audio_and_one_use() {
        let now = DateTime::parse_from_rfc3339("2026-11-22T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let body = (google().body)(google(), now, &json!({}));
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

    /// The token carries what the page asked for — and only what a page may
    /// ask: its instruction, transcripts, resumption and compression stay;
    /// tools, another model and another output do not.
    #[test]
    fn the_token_carries_the_pages_setup_cut_to_what_a_page_may_ask() {
        let now = DateTime::parse_from_rfc3339("2026-11-22T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let asked = json!({
            "model": "models/another-model",
            "generationConfig": { "responseModalities": ["TEXT"], "temperature": 2 },
            "systemInstruction": { "parts": [{ "text": "You are a teacher." }] },
            "inputAudioTranscription": {},
            "outputAudioTranscription": {},
            "sessionResumption": { "handle": "h1" },
            "contextWindowCompression": { "triggerTokens": 6000, "slidingWindow": { "targetTokens": 3000 } },
            "tools": [{ "googleSearch": {} }],
            "realtimeInputConfig": { "x": 1 },
        });
        let body = (google().body)(google(), now, &asked);
        let setup = body["bidiGenerateContentSetup"].as_object().unwrap();
        let mut names: Vec<&str> = setup.keys().map(String::as_str).collect();
        names.sort_unstable();
        let mut allowed: Vec<&str> = GOOGLE_PAGE_MAY_ASK.to_vec();
        allowed.extend(["model", "generationConfig"]);
        allowed.sort_unstable();
        assert_eq!(names, allowed, "nothing but what a page may ask, plus the engine's two");
        assert_eq!(setup["model"], "models/gemini-3.8-live", "the engine's model, not the page's");
        assert_eq!(setup["generationConfig"], json!({ "responseModalities": ["AUDIO"] }));
        assert_eq!(setup["systemInstruction"], asked["systemInstruction"]);
        assert_eq!(setup["sessionResumption"], json!({ "handle": "h1" }));
        assert!(!GOOGLE_PAGE_MAY_ASK.contains(&"tools"), "a live voice gets words, not tools");
        // A setup that is not an object asks for nothing.
        let bare = (google().body)(google(), now, &json!("x"));
        assert_eq!(bare["bidiGenerateContentSetup"].as_object().unwrap().len(), 2);
    }

    fn alibaba() -> &'static Provider {
        provider_named("alibaba").unwrap()
    }

    /// The list's order is the preference order, and Qwen is first.
    #[test]
    fn the_first_provider_listed_is_the_one_preferred() {
        let ids: Vec<&str> = PROVIDERS.iter().map(|p| p.id).collect();
        assert_eq!(ids, ["alibaba", "google"]);
    }

    /// Qwen's token request: a bearer header, the lifetime in the query, no
    /// body — and the page's setup, whatever it holds, is not sent.
    #[test]
    fn qwens_token_request_has_a_bearer_header_a_lifetime_and_no_body() {
        let p = alibaba();
        assert_eq!(p.token_path, "/api/v1/tokens?expire_in_seconds=60");
        assert_eq!((p.key_header, p.key_prefix), ("Authorization", "Bearer "));
        let now = Utc::now();
        let asked = json!({ "systemInstruction": "x", "tools": [1], "anything": true });
        assert!((p.body)(p, now, &asked).is_null(), "a page's setup is dropped");
        assert!((p.body)(p, now, &json!({})).is_null());
    }

    #[test]
    fn qwens_answer_needs_a_token_and_names_its_end_in_unix_seconds() {
        let ok = json!({"token": "st-abc", "expires_at": 1_800_000_000});
        let read = (alibaba().read)(&ok).unwrap();
        assert_eq!(read.token.expose(), "st-abc");
        assert_eq!(read.expires.as_deref(), Some("2027-01-15T08:00:00Z"));
        let bare = (alibaba().read)(&json!({"token": "st-abc"})).unwrap();
        assert_eq!(bare.expires, None);
        assert!((alibaba().read)(&json!({"expires_at": 1_800_000_000})).is_none());
        assert!((alibaba().read)(&json!({"token": ""})).is_none());
        let refusal = json!({"code": "InvalidApiKey", "message": "m", "request_id": "r"});
        assert!((alibaba().read)(&refusal).is_none());
    }

    #[test]
    fn qwens_address_is_complete_with_the_model_and_the_short_key_in_the_query() {
        let url = (alibaba().socket_with)(alibaba(), &Secret::new("st-abc"));
        assert_eq!(
            url,
            "wss://ap-southeast-1.maas.qwencloudapi.com/api-ws/v1/realtime?model=qwen-audio-3.1-realtime-flash&api_key=st-abc"
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
            max_minutes: None,
            lessons_left: None,
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
            3,
            "the key's header and each provider's socket address open a secret"
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

    /// Who pays is decided in one function: the person's own key beats Linggen
    /// Cloud, and a service the site holds no key for is never Cloud's.
    #[test]
    fn the_source_is_the_own_key_first_then_linggen_cloud() {
        assert!(alibaba().via_cloud() && !google().via_cloud());
        let own = Source::OwnKey(Secret::new("k"));
        assert_eq!((own.as_str(), Source::LinggenCloud.as_str()), ("own_key", "linggen_cloud"));
        let code = include_str!("live.rs");
        let production = &code[..code.find("#[cfg(test)]").unwrap()];
        assert_eq!(production.matches("fn find_source(").count(), 1, "who pays is decided in one function");
        let from = production.find("pub fn find_source(").unwrap();
        let decision = &production[from..production[from..].find("\n}\n").map(|n| from + n).unwrap()];
        assert!(
            decision.find("find_key(").unwrap() < decision.find("live_usable(").unwrap(),
            "the own key is asked before Linggen Cloud"
        );
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
            Refusal::NotSubscribed,
            Refusal::LessonsUsed,
            Refusal::DailyCap,
        ];
        let words: std::collections::HashSet<_> = all.iter().map(|r| r.code()).collect();
        assert_eq!(words.len(), all.len());
        assert!(all
            .iter()
            .all(|r| matches!(r.status(), 400 | 402 | 409 | 429 | 502)));
    }
}

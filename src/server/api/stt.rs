//! `POST /api/stt` — hear the person: what they said to her, as words.
//!
//! Hearing sits behind the [`SttProvider`] trait, as her voice sits behind
//! `TtsProvider`: the engine that turns sound into words is a swap, not a
//! rewrite. The daemon's hearing is Whisper on MLX in the voice lane's
//! sidecar ([`super::stt_mlx`]).
//!
//! `GET /api/stt` — can she hear now? `{"hears": true}`, or `{"hears": false,
//! "reason", "fetching"}`, asked before the microphone opens. It is the same
//! judgement the POST makes ([`hears_now`], the only place that asks the
//! provider), so the two cannot disagree; asking starts nothing.
//!
//! The body is one clip, WAV, 16 kHz mono 16-bit PCM, at most 60 seconds —
//! raw (`audio/wav`), or `{"wav_b64": "…"}` as JSON, the form a request on
//! the data channel can carry (it sends every body as JSON). The answer is
//! `{text, language, ms}`; nothing intelligible is `{"text": ""}`; a machine
//! that cannot hear is 503 `{error: "unavailable", reason, fetching}`, the
//! reason in words a person can read — said before the clip is looked at, so
//! a silent clip on such a machine gets the reason, not "Didn't catch that".
//!
//! The sound is only ever heard, never kept: the provider is handed samples
//! (no path, no URL), nothing here writes a file or opens a connection, and
//! no line of the log carries it (`a_hearing_has_no_way_to_keep_or_send_the_sound`).
//! The words are kept as any line the person speaks to her is — by her turn,
//! not here.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine as _;
use serde_json::json;

use crate::engine::permission::PermissionMode;
use crate::server::ServerState;

/// The one rate a clip is heard at; the surface makes it so.
pub(crate) const RATE: u32 = 16_000;
/// The longest clip, seconds.
pub(crate) const MAX_SECS: usize = 60;
/// The most of a request body this route reads: a minute of audio, as the
/// JSON form's base64 carries it, with room.
pub(crate) const BODY_LIMIT: usize = 3 * 1024 * 1024;

/// A clip shorter than this holds no sentence.
const MIN_SAMPLES: usize = RATE as usize / 4;
/// A clip whose loudest 20 ms is quieter than this (RMS, 1.0 = full scale;
/// about -48 dBFS — an estimate, to be tuned on a real microphone: speech
/// sits some 20 to 30 dB above it, a quiet room's hiss below) holds no speech. It is never put to the model: Whisper
/// invents words from silence.
const SILENCE_RMS: f32 = 0.004;
const FRAME: usize = RATE as usize / 50;

/// What was heard.
#[derive(Debug, Clone, PartialEq)]
pub struct Heard {
    pub text: String,
    /// The language the model took the clip to be in (`zh`, `en`, …).
    pub language: String,
}

/// This machine cannot hear now. The reason is words for the person, not for
/// a log: what stops it, or what is being done about it. `fetching`: what
/// hearing waits on (the model's fetch, or its load) is under way, so trying
/// again soon will do; false after a failure, where the next press tries again.
#[derive(Debug, Clone, PartialEq)]
pub struct Unavailable {
    pub reason: String,
    pub fetching: bool,
}

impl Unavailable {
    pub fn because(reason: impl Into<String>) -> Self {
        Self { reason: reason.into(), fetching: false }
    }

    pub fn fetching(reason: impl Into<String>) -> Self {
        Self { reason: reason.into(), fetching: true }
    }

    /// The fields a person's page reads, in both answers that carry a no.
    fn fields(&self) -> serde_json::Value {
        json!({ "reason": self.reason, "fetching": self.fetching })
    }
}

/// A hearing backend: samples in, words out. 16 kHz mono 16-bit, at most
/// [`MAX_SECS`] and never silent — the route has judged that before the call.
/// There is no path or URL in this interface: a backend is handed the sound,
/// the language tag the clip is in ([`language_to_hear`]) and nothing to
/// fetch it from or send it to.
#[async_trait]
pub trait SttProvider: Send + Sync {
    /// Can it hear now? Asking starts nothing and changes nothing: this is
    /// the one judgement behind both the GET's answer and the POST's 503.
    fn can_hear(&self) -> Result<(), Unavailable>;
    /// Start fetching what hearing waits on, if it waits on something that
    /// can be fetched and no fetch runs. Only a clip sent (a real press)
    /// calls this; the GET never does.
    fn begin_fetch(&self);
    /// Load what hearing needs now if it is already on this machine, never
    /// fetching anything: what a start does so the first press does not wait
    /// (`background::prewarm_voice`).
    fn warm_if_fetched(&self);
    async fn hear(&self, samples: &[i16], language: &str) -> Result<Heard, Unavailable>;
}

/// The daemon's hearing — the single switch point for the daemon's STT.
pub fn default_provider(sidecar: Arc<super::mlx_sidecar::VoiceSidecar>) -> Arc<dyn SttProvider> {
    Arc::new(super::stt_mlx::MlxSttProvider::new(sidecar))
}

/// GET /api/stt — can she hear now, before a microphone is opened. Owner
/// only, as the POST is; classed as it is.
pub(crate) async fn stt_status_handler(State(state): State<Arc<ServerState>>, headers: HeaderMap) -> Response {
    answer_status(state.stt.as_ref(), crate::server::api::pair::caller_level(&headers))
}

fn answer_status(stt: &dyn SttProvider, level: PermissionMode) -> Response {
    if let Some(refused) = crate::server::person::owners_only(level) {
        return refused;
    }
    match hears_now(stt) {
        Ok(()) => Json(json!({ "hears": true })).into_response(),
        Err(no) => {
            let mut body = no.fields();
            body["hears"] = json!(false);
            Json(body).into_response()
        }
    }
}

/// The one judgement of whether she can hear, asked of the provider here
/// and nowhere else (`hearing_is_judged_in_one_place`).
fn hears_now(stt: &dyn SttProvider) -> Result<(), Unavailable> {
    stt.can_hear()
}

/// POST /api/stt — a clip in, `{text, language, ms}` out. Her own thread
/// runs with the owner's tools, so hearing is the owner's too (as her avatar
/// chat is, `yinyue::chat_handler`); the route's class is `/api/tts`'s.
pub(crate) async fn stt_handler(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let level = crate::server::api::pair::caller_level(&headers);
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    answer(state.stt.as_ref(), (level, content_type), language_to_hear(&state), &body).await
}

/// The language a clip is in, as far as the engine knows it — the one place
/// that decides it. Two sources, the first that has one: the person's own
/// setting (Settings → Language); under Auto, the language of the line they
/// last typed to her (`resident::language_person_typed_to_her`). With
/// neither the tag is empty and the model finds the language itself: the
/// Mac's language is not evidence of what a person speaks, and a clip forced
/// into the wrong language comes back as nonsense.
async fn language_to_hear(state: &Arc<ServerState>) -> String {
    let setting = crate::language::setting();
    let written = match setting {
        Some(_) => None,
        None => crate::server::resident::language_person_typed_to_her(state).await,
    };
    heard_in(setting.as_deref(), written.as_deref())
}

/// [`language_to_hear`]'s rule, on what it reads.
fn heard_in(setting: Option<&str>, written: Option<&str>) -> String {
    setting.or(written).unwrap_or("").to_string()
}

/// The route's whole decision, on what it is handed (so a test needs no
/// server): who asks, then which of the two body forms it is (a third is
/// refused before anything starts), then whether this machine can hear at
/// all (a press that finds it cannot starts the fetch it waits on), then
/// what the body holds, whether that is speech, and only then the model.
/// `language` is the clip's language, awaited at that last step alone: a
/// request refused or answered before it reads nothing of her thread.
async fn answer(
    stt: &dyn SttProvider,
    (level, content_type): (PermissionMode, &str),
    language: impl std::future::Future<Output = String>,
    body: &[u8],
) -> Response {
    if let Some(refused) = crate::server::person::owners_only(level) {
        return refused;
    }
    let Some(form) = BodyForm::of(content_type) else {
        return unsupported_media_type();
    };
    // Before the clip is looked at: a silent clip on a machine that cannot
    // hear gets the reason, as a loud one does.
    stt.begin_fetch();
    if let Err(no) = hears_now(stt) {
        return unavailable(&no);
    }
    let samples = match samples_of(form, body) {
        Ok(samples) => samples,
        Err(refusal) => return refusal.into_response(),
    };
    if holds_no_speech(&samples) {
        return Json(json!({ "text": "" })).into_response();
    }
    let started = Instant::now();
    match stt.hear(&samples, &language.await).await {
        Ok(heard) if heard.text.trim().is_empty() => Json(json!({ "text": "" })).into_response(),
        Ok(heard) => {
            let ms = started.elapsed().as_millis() as u64;
            // The length, never the words: what was said is the person's.
            tracing::info!("[stt] heard {} chars in {ms} ms", heard.text.chars().count());
            Json(json!({ "text": heard.text.trim(), "language": heard.language, "ms": ms })).into_response()
        }
        Err(no) => unavailable(&no),
    }
}

/// The two forms a request body takes: the clip itself (`audio/wav`), or
/// `{"wav_b64": …}` (`application/json`, which a data-channel request is).
/// Nothing else — a `text/plain` form post is the one body a foreign page can
/// send without asking first — and the check is on the media type itself.
#[derive(Clone, Copy, Debug, PartialEq)]
enum BodyForm {
    Wav,
    Json,
}

impl BodyForm {
    fn of(content_type: &str) -> Option<Self> {
        let media_type = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        match media_type.as_str() {
            "audio/wav" => Some(Self::Wav),
            "application/json" => Some(Self::Json),
            _ => None,
        }
    }
}

fn unsupported_media_type() -> Response {
    (
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        Json(json!({ "error": "unsupported_media_type", "reason": "Send audio/wav, or application/json with wav_b64." })),
    )
        .into_response()
}

fn unavailable(no: &Unavailable) -> Response {
    let mut body = no.fields();
    body["error"] = json!("unavailable");
    (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
}

/// A request refused for what its body is.
#[derive(Debug, PartialEq)]
enum Refusal {
    /// Not a clip in the one form heard.
    NotThatWav(&'static str),
    TooLong,
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        match self {
            Refusal::NotThatWav(reason) => {
                (StatusCode::BAD_REQUEST, Json(json!({ "error": "bad_audio", "reason": reason }))).into_response()
            }
            Refusal::TooLong => (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(json!({ "error": "too_long", "reason": format!("At most {MAX_SECS} seconds.") })),
            )
                .into_response(),
        }
    }
}

/// The clip's samples, from the form it came in.
fn samples_of(form: BodyForm, body: &[u8]) -> Result<Vec<i16>, Refusal> {
    #[derive(serde::Deserialize)]
    struct Json {
        wav_b64: String,
    }
    let decoded;
    let wav = match form == BodyForm::Json {
        true => {
            let parsed: Json = serde_json::from_slice(body)
                .map_err(|_| Refusal::NotThatWav("expected a WAV clip, or {\"wav_b64\": …}"))?;
            decoded = base64::engine::general_purpose::STANDARD
                .decode(parsed.wav_b64)
                .map_err(|_| Refusal::NotThatWav("wav_b64 is not base64"))?;
            &decoded[..]
        }
        false => body,
    };
    let samples = super::tts_wav::mono_pcm16(wav, RATE)
        .map_err(|_| Refusal::NotThatWav("expected a 16 kHz mono 16-bit PCM WAV clip"))?;
    match samples.len() > MAX_SECS * RATE as usize {
        true => Err(Refusal::TooLong),
        false => Ok(samples),
    }
}

/// Too short to be a sentence, or never louder than a quiet room.
fn holds_no_speech(samples: &[i16]) -> bool {
    if samples.len() < MIN_SAMPLES {
        return true;
    }
    let loudest = samples
        .chunks(FRAME)
        .map(|frame| {
            let sum: f32 = frame.iter().map(|&s| (s as f32 / 32768.0).powi(2)).sum();
            (sum / frame.len() as f32).sqrt()
        })
        .fold(0.0, f32::max);
    loudest < SILENCE_RMS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::api::tts_wav::{self, Pcm};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Hears what it was told to, counting the calls.
    struct Fake {
        says: Result<Heard, Unavailable>,
        /// What `can_hear` answers.
        verdict: Result<(), Unavailable>,
        calls: AtomicUsize,
        fetches: AtomicUsize,
        heard_samples: AtomicUsize,
        /// The language tag of the last clip it was handed.
        heard_in: std::sync::Mutex<Option<String>>,
    }

    impl Fake {
        fn saying(text: &str, language: &str) -> Self {
            let says = Ok(Heard { text: text.into(), language: language.into() });
            Fake {
                says,
                verdict: Ok(()),
                calls: AtomicUsize::new(0),
                fetches: AtomicUsize::new(0),
                heard_samples: AtomicUsize::new(0),
                heard_in: std::sync::Mutex::new(None),
            }
        }
        /// A machine that cannot hear, for a reason that will not pass.
        fn deaf(reason: &str) -> Self {
            Fake { verdict: Err(Unavailable::because(reason)), ..Fake::saying("", "") }
        }
        /// A machine whose hearing model is on its way.
        fn fetching(reason: &str) -> Self {
            Fake { verdict: Err(Unavailable::fetching(reason)), ..Fake::saying("", "") }
        }
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
        fn fetches(&self) -> usize {
            self.fetches.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl SttProvider for Fake {
        fn can_hear(&self) -> Result<(), Unavailable> {
            self.verdict.clone()
        }
        fn begin_fetch(&self) {
            self.fetches.fetch_add(1, Ordering::SeqCst);
        }
        fn warm_if_fetched(&self) {}
        async fn hear(&self, samples: &[i16], language: &str) -> Result<Heard, Unavailable> {
            *self.heard_in.lock().unwrap() = Some(language.to_string());
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.heard_samples.store(samples.len(), Ordering::SeqCst);
            self.says.clone()
        }
    }

    /// `secs` of a 220 Hz tone at about -20 dBFS.
    fn speech(secs: f32) -> Vec<i16> {
        (0..(RATE as f32 * secs) as usize)
            .map(|i| ((i as f32 * 220.0 * std::f32::consts::TAU / RATE as f32).sin() * 3000.0) as i16)
            .collect()
    }

    fn wav(samples: Vec<i16>) -> Vec<u8> {
        tts_wav::encode(&Pcm { rate: RATE, samples })
    }

    const OWNER: PermissionMode = PermissionMode::Admin;

    async fn ask(stt: &Fake, level: PermissionMode, content_type: &str, body: &[u8]) -> (StatusCode, serde_json::Value) {
        let response = answer(stt, (level, content_type), async { "en".to_string() }, body).await;
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
    }

    #[tokio::test]
    async fn a_wav_in_is_the_fakes_words_out() {
        let fake = Fake::saying("今天有什么安排 and my calendar", "zh");
        let (status, body) = ask(&fake, OWNER, "audio/wav", &wav(speech(1.0))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["text"], "今天有什么安排 and my calendar");
        assert_eq!(body["language"], "zh");
        assert!(body["ms"].is_u64(), "{body}");
        assert_eq!(fake.heard_samples.load(Ordering::SeqCst), RATE as usize, "the model got the clip's samples");
    }

    /// The data channel sends every body as JSON: the same clip, the same words.
    #[tokio::test]
    async fn the_json_form_of_the_clip_is_heard_the_same() {
        let fake = Fake::saying("hello", "en");
        let b64 = base64::engine::general_purpose::STANDARD.encode(wav(speech(1.0)));
        let body = json!({ "wav_b64": b64, "user_id": "owner", "user_type": "paired" });
        let (status, heard) = ask(&fake, OWNER, "application/json", body.to_string().as_bytes()).await;
        assert_eq!((status, heard["text"].as_str()), (StatusCode::OK, Some("hello")));
        let (status, _) = ask(&fake, OWNER, "application/json", b"{\"wav_b64\": 3}").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    /// Nothing intelligible is `{"text": ""}` and the model is never asked:
    /// it would invent words from silence.
    #[tokio::test]
    async fn silence_is_empty_text_and_never_reaches_the_model() {
        let fake = Fake::saying("Thank you for watching.", "en");
        let clips = [
            vec![0i16; RATE as usize * 2],
            // A room's hiss: about -60 dBFS.
            speech(2.0).iter().map(|s| s / 50).collect(),
            // A click.
            speech(0.1),
        ];
        for clip in clips {
            let (status, body) = ask(&fake, OWNER, "audio/wav", &wav(clip)).await;
            assert_eq!((status, body), (StatusCode::OK, json!({ "text": "" })));
        }
        assert_eq!(fake.calls(), 0);
    }

    /// Only the two body forms are taken, and a third is refused before the
    /// press begins a fetch or a hearing (a cross-site `text/plain` form post
    /// is the one body a page can send without asking).
    #[tokio::test]
    async fn only_the_two_body_forms_are_taken_and_a_third_starts_nothing() {
        let fake = Fake::saying("ok", "en");
        for content_type in ["text/plain", "", "application/jsonx", "audio/mpeg", "multipart/form-data", "audio/x-wav", "application/octet-stream"] {
            let (status, body) = ask(&fake, OWNER, content_type, &wav(speech(1.0))).await;
            assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{content_type:?}");
            assert_eq!(body["error"], "unsupported_media_type", "{content_type:?}");
        }
        assert_eq!((fake.fetches(), fake.calls()), (0, 0));
        // The two forms, in any case and with parameters, are taken.
        let b64 = base64::engine::general_purpose::STANDARD.encode(wav(speech(1.0)));
        let json_body = json!({ "wav_b64": b64 }).to_string();
        for (content_type, body) in [("Audio/WAV", wav(speech(1.0))), ("application/json; charset=utf-8", json_body.into_bytes())] {
            assert_eq!(ask(&fake, OWNER, content_type, &body).await.0, StatusCode::OK, "{content_type}");
        }
        // The knock a page sends to start a fetch is JSON and is taken.
        let deaf = Fake::fetching("Fetching the hearing model (1.6 GB). Try again when it is done.");
        assert_eq!(ask(&deaf, OWNER, "application/json", b"{}").await.0, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(deaf.fetches(), 1);
    }

    #[tokio::test]
    async fn words_the_model_could_not_make_are_empty_text() {
        let fake = Fake::saying("  ", "en");
        let (status, body) = ask(&fake, OWNER, "audio/wav", &wav(speech(1.0))).await;
        assert_eq!((status, body), (StatusCode::OK, json!({ "text": "" })));
    }

    #[tokio::test]
    async fn a_clip_over_sixty_seconds_is_refused_and_sixty_is_not() {
        let fake = Fake::saying("ok", "en");
        let (status, body) = ask(&fake, OWNER, "audio/wav", &wav(speech(60.5))).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(body["error"], "too_long");
        assert_eq!(fake.calls(), 0);
        let (status, _) = ask(&fake, OWNER, "audio/wav", &wav(speech(60.0))).await;
        assert_eq!(status, StatusCode::OK);
        // A minute of audio is a body this route reads (its own limit, not the framework's).
        assert!(wav(speech(60.0)).len() < BODY_LIMIT);
    }

    #[tokio::test]
    async fn only_a_16k_mono_16_bit_wav_is_heard() {
        let fake = Fake::saying("ok", "en");
        let stereo = {
            let mut w = wav(speech(1.0));
            w[22] = 2; // channels
            w
        };
        let other_rate = tts_wav::encode(&Pcm { rate: 44_100, samples: speech(1.0) });
        for (what, body) in [
            ("plain text", b"hello".to_vec()),
            ("empty", Vec::new()),
            ("stereo", stereo),
            ("44.1 kHz", other_rate),
            ("truncated header", wav(speech(1.0))[..30].to_vec()),
        ] {
            let (status, body) = ask(&fake, OWNER, "audio/wav", &body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{what}");
            assert_eq!(body["error"], "bad_audio", "{what}");
        }
        assert_eq!(fake.calls(), 0);
    }

    /// Her own thread is the owner's: a device below admin is refused before
    /// the body is read, and the model is never asked.
    #[tokio::test]
    async fn only_the_owner_is_heard() {
        let fake = Fake::saying("ok", "en");
        for level in [PermissionMode::Chat, PermissionMode::Read, PermissionMode::Edit] {
            let (status, body) = ask(&fake, level, "audio/wav", &wav(speech(1.0))).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{level}");
            assert_eq!(body["error"], "beyond_level", "{level}");
        }
        assert_eq!(fake.calls(), 0);
        assert_eq!(ask(&fake, OWNER, "audio/wav", &wav(speech(1.0))).await.0, StatusCode::OK);
    }

    #[tokio::test]
    async fn a_machine_that_cannot_hear_says_why_in_a_few_words() {
        let reason = "Hearing needs an Apple Silicon Mac.";
        let fake = Fake::deaf(reason);
        let (status, body) = ask(&fake, OWNER, "audio/wav", &wav(speech(1.0))).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body, json!({ "error": "unavailable", "reason": reason, "fetching": false }));
        assert_eq!(fake.calls(), 0);
    }

    /// Rule 1: the reason is not kept behind the silence check. Every clip a
    /// deaf machine is sent — silent, a click, a bad body — gets it.
    #[tokio::test]
    async fn a_silent_clip_on_a_machine_that_cannot_hear_gets_the_reason_not_a_shrug() {
        let fake = Fake::deaf("Hearing needs an Apple Silicon Mac.");
        for clip in [vec![0i16; RATE as usize * 2], speech(0.1)] {
            let (status, body) = ask(&fake, OWNER, "audio/wav", &wav(clip)).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(body["reason"], "Hearing needs an Apple Silicon Mac.");
        }
        let (status, _) = ask(&fake, OWNER, "audio/wav", b"").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "an empty body is a press knocking, not a bad clip");
        assert_eq!(fake.calls(), 0);
    }

    /// A press that finds the model missing starts the fetch it waits on:
    /// the clip sent is how a real press begins it (the GET never does).
    #[tokio::test]
    async fn a_press_that_finds_no_model_starts_the_fetch_once_per_press() {
        let fake = Fake::fetching("Fetching the hearing model (1.6 GB). Try again when it is done.");
        let (status, body) = ask(&fake, OWNER, "audio/wav", &wav(speech(1.0))).await;
        assert_eq!((status, body["fetching"].as_bool()), (StatusCode::SERVICE_UNAVAILABLE, Some(true)));
        assert_eq!(fake.fetches(), 1);
        // A refused caller starts nothing.
        ask(&fake, PermissionMode::Chat, "audio/wav", &wav(speech(1.0))).await;
        assert_eq!(fake.fetches(), 1);
    }

    async fn status_of(stt: &Fake, level: PermissionMode) -> (StatusCode, serde_json::Value) {
        let response = answer_status(stt, level);
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
    }

    #[tokio::test]
    async fn asking_whether_she_hears_says_yes_or_the_reason_and_starts_nothing() {
        let hears = Fake::saying("ok", "en");
        assert_eq!(status_of(&hears, OWNER).await, (StatusCode::OK, json!({ "hears": true })));
        let deaf = Fake::deaf("Hearing needs an Apple Silicon Mac.");
        let (status, body) = status_of(&deaf, OWNER).await;
        assert_eq!(
            (status, body),
            (StatusCode::OK, json!({ "hears": false, "reason": "Hearing needs an Apple Silicon Mac.", "fetching": false }))
        );
        let fetching = Fake::fetching("Fetching the hearing model (1.6 GB). Try again when it is done.");
        let (_, body) = status_of(&fetching, OWNER).await;
        assert_eq!((body["hears"].as_bool(), body["fetching"].as_bool()), (Some(false), Some(true)));
        for fake in [&hears, &deaf, &fetching] {
            assert_eq!(fake.fetches() + fake.calls(), 0, "asking started something");
        }
    }

    /// The answer the GET gives and the POST's are one judgement: for every
    /// state a machine can be in, `hears` is true exactly when a clip is
    /// heard, and a no carries the same reason and `fetching`.
    #[tokio::test]
    async fn the_gets_answer_and_the_posts_503_never_disagree() {
        let states = [
            Fake::saying("ok", "en"),
            Fake::deaf("Hearing needs an Apple Silicon Mac."),
            Fake::fetching("Fetching the hearing model (1.6 GB). Try again when it is done."),
        ];
        for fake in &states {
            let (_, got) = status_of(fake, OWNER).await;
            let (status, posted) = ask(fake, OWNER, "audio/wav", &wav(speech(1.0))).await;
            assert_eq!(got["hears"].as_bool().unwrap(), status == StatusCode::OK, "{got} vs {status}");
            if status == StatusCode::SERVICE_UNAVAILABLE {
                assert_eq!((&got["reason"], &got["fetching"]), (&posted["reason"], &posted["fetching"]));
            }
        }
    }

    #[tokio::test]
    async fn only_the_owner_is_told_whether_she_hears() {
        let fake = Fake::saying("ok", "en");
        for level in [PermissionMode::Chat, PermissionMode::Read, PermissionMode::Edit] {
            let (status, body) = status_of(&fake, level).await;
            assert_eq!((status, body["error"].as_str()), (StatusCode::FORBIDDEN, Some("beyond_level")), "{level}");
        }
    }

    /// Rule 2: the clip is heard in the language the engine knows the person
    /// to speak: the setting; under Auto the language of what they last typed
    /// to her. With neither (the first press of a day, an English line) no
    /// language is told and the model finds it — never the Mac's.
    #[test]
    fn a_clip_is_heard_in_the_language_the_engine_knows() {
        // setting, typed line's language -> the tag passed on
        let cases = [
            (Some("en"), None, "en"),
            (Some("zh-Hant"), Some("zh-Hans"), "zh-Hant"),
            (None, Some("zh-Hans"), "zh-Hans"),
            (None, None, ""),
        ];
        for (setting, written, want) in cases {
            assert_eq!(heard_in(setting, written), want, "{setting:?} {written:?}");
        }
    }

    /// The route hands the provider the language it was given, and only that.
    #[tokio::test]
    async fn the_route_hands_the_clips_language_to_the_hearing() {
        let fake = Fake::saying("今天有什么安排", "zh");
        let body = wav(speech(1.0));
        for tag in ["zh-Hans", "en", "ja"] {
            let response = answer(&fake, (OWNER, "audio/wav"), async { tag.to_string() }, &body).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(fake.heard_in.lock().unwrap().as_deref(), Some(tag));
        }
    }

    /// What is refused or answered before the model is asked never reads her
    /// thread for the language: the future is awaited only at the hear.
    #[tokio::test]
    async fn the_language_is_read_only_for_a_clip_that_is_heard() {
        let polled = AtomicUsize::new(0);
        async fn asked(stt: &Fake, polled: &AtomicUsize, level: PermissionMode, content_type: &str, body: &[u8]) -> StatusCode {
            let language = async {
                polled.fetch_add(1, Ordering::SeqCst);
                "en".to_string()
            };
            answer(stt, (level, content_type), language, body).await.status()
        }
        let hears = Fake::saying("ok", "en");
        let clip = wav(speech(1.0));
        assert_eq!(asked(&hears, &polled, PermissionMode::Chat, "audio/wav", &clip).await, StatusCode::FORBIDDEN);
        assert_eq!(asked(&hears, &polled, OWNER, "text/plain", &clip).await, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert_eq!(asked(&hears, &polled, OWNER, "audio/wav", b"hello").await, StatusCode::BAD_REQUEST);
        assert_eq!(asked(&hears, &polled, OWNER, "audio/wav", &wav(vec![0i16; RATE as usize * 2])).await, StatusCode::OK, "silence");
        assert_eq!(asked(&Fake::deaf("no"), &polled, OWNER, "audio/wav", &clip).await, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(polled.load(Ordering::SeqCst), 0, "her thread was read for a clip that was never heard");
        assert_eq!(asked(&hears, &polled, OWNER, "audio/wav", &clip).await, StatusCode::OK);
        assert_eq!(polled.load(Ordering::SeqCst), 1);
    }

    /// Lock: the clip's language is decided in `language_to_hear` and nowhere
    /// below it: the hearing backend reads no language of its own.
    #[test]
    fn the_clips_language_is_decided_in_one_place() {
        let mlx = include_str!("stt_mlx.rs");
        let production = &mlx[..mlx.find("#[cfg(test)]").unwrap()];
        for read in ["language::setting", "language::effective", "language_person_typed_to_her"] {
            assert!(!production.contains(read), "stt_mlx.rs reads `{read}`: take the tag `hear` is handed");
        }
        let code = include_str!("stt.rs");
        let production = &code[..code.find("#[cfg(test)]").unwrap()];
        assert_eq!(production.matches("language::setting()").count(), 1, "the setting is read in language_to_hear alone");
    }

    /// Lock: the provider is asked whether it hears in one place, the
    /// function both routes call, and the fetch is begun only by a clip sent.
    #[test]
    fn hearing_is_judged_in_one_place() {
        let code = include_str!("stt.rs");
        let production = &code[..code.find("#[cfg(test)]").unwrap()];
        assert_eq!(production.matches("stt.can_hear()").count(), 1, "ask through hears_now alone");
        assert_eq!(production.matches("hears_now(").count(), 3, "the function, the GET's answer, the POST");
        assert_eq!(production.matches("stt.begin_fetch()").count(), 1, "only a clip sent begins a fetch");
        let status = &production[production.find("fn stt_status_handler").unwrap()..production.find("fn hears_now").unwrap()];
        assert!(!status.contains("begin_fetch"), "asking whether she hears starts nothing");
    }

    /// The route is `/api/tts`'s class, and her door's: any device may reach
    /// it, the handler keeps it to the owner.
    #[test]
    fn the_route_is_classed_as_her_voice_is() {
        use crate::server::routes::classes::class_of;
        assert_eq!(class_of("POST", "/api/stt"), class_of("POST", "/api/tts"));
        assert_eq!(class_of("GET", "/api/stt"), class_of("POST", "/api/tts"));
        assert_eq!(class_of("POST", "/api/stt").needs(), PermissionMode::Chat);
    }

    /// Nothing that hears can keep or send what it hears: the hearing modules
    /// open no file, no connection and no temp file, and name no network
    /// type. (The sidecar script's own half is `stt_mlx`'s lock.)
    #[test]
    fn a_hearing_has_no_way_to_keep_or_send_the_sound() {
        let banned = [
            "reqwest", "TcpStream", "UdpSocket", "hyper::", "tokio::fs", "std::fs", "tempfile",
            "File::", "OpenOptions", "std::net",
        ];
        let mut seen = 0;
        for (rel, code) in crate::server::lock_scan::production_code(&["src/server/api"]) {
            if !(rel.ends_with("api/stt.rs") || rel.ends_with("api/stt_mlx.rs")) {
                continue;
            }
            seen += 1;
            for word in banned {
                assert!(!code.contains(word), "{rel} names `{word}`: what is heard is never kept or sent");
            }
        }
        assert_eq!(seen, 2, "the hearing modules moved: update this lock");
    }

    /// The provider is handed the sound and the clip's language tag, nothing else: samples, no path or URL.
    #[test]
    fn a_hearing_backend_is_handed_samples_alone() {
        let code = include_str!("stt.rs");
        assert!(code.contains("async fn hear(&self, samples: &[i16], language: &str) -> Result<Heard, Unavailable>;"));
    }
}

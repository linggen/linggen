//! `POST /api/stt` — hear the person: what they said to her, as words.
//!
//! Hearing sits behind the [`SttProvider`] trait, as her voice sits behind
//! `TtsProvider`: the engine that turns sound into words is a swap, not a
//! rewrite. The daemon's hearing is Whisper on MLX in the voice lane's
//! sidecar ([`super::stt_mlx`]).
//!
//! The body is one clip, WAV, 16 kHz mono 16-bit PCM, at most 60 seconds —
//! raw (`audio/wav`), or `{"wav_b64": "…"}` as JSON, the form a request on
//! the data channel can carry (it sends every body as JSON). The answer is
//! `{text, language, ms}`; nothing intelligible is `{"text": ""}`; a machine
//! that cannot hear is 503 `{error: "unavailable", reason}`, the reason in
//! words a person can read.
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
/// about -48 dBFS, below a quiet room's own noise floor on a laptop
/// microphone) holds no speech. It is never put to the model: Whisper
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
/// a log: what stops it, or what is being done about it.
#[derive(Debug, Clone, PartialEq)]
pub struct Unavailable(pub String);

/// A hearing backend: samples in, words out. 16 kHz mono 16-bit, at most
/// [`MAX_SECS`] and never silent — the route has judged that before the call.
/// There is no path or URL in this interface: a backend is handed the sound
/// and nothing to fetch it from or send it to.
#[async_trait]
pub trait SttProvider: Send + Sync {
    async fn hear(&self, samples: &[i16]) -> Result<Heard, Unavailable>;
}

/// The daemon's hearing — the single switch point for the daemon's STT.
pub fn default_provider(sidecar: Arc<super::mlx_sidecar::VoiceSidecar>) -> Arc<dyn SttProvider> {
    Arc::new(super::stt_mlx::MlxSttProvider::new(sidecar))
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
    answer(state.stt.as_ref(), (level, content_type), &body).await
}

/// The route's whole decision, on what it is handed (so a test needs no
/// server): who asks, then what the body is, then whether it holds speech,
/// and only then the model.
async fn answer(stt: &dyn SttProvider, (level, content_type): (PermissionMode, &str), body: &[u8]) -> Response {
    if let Some(refused) = crate::server::person::owners_only(level) {
        return refused;
    }
    let samples = match samples_of(content_type, body) {
        Ok(samples) => samples,
        Err(refusal) => return refusal.into_response(),
    };
    if holds_no_speech(&samples) {
        return Json(json!({ "text": "" })).into_response();
    }
    let started = Instant::now();
    match stt.hear(&samples).await {
        Ok(heard) if heard.text.trim().is_empty() => Json(json!({ "text": "" })).into_response(),
        Ok(heard) => {
            let ms = started.elapsed().as_millis() as u64;
            // The length, never the words: what was said is the person's.
            tracing::info!("[stt] heard {} chars in {ms} ms", heard.text.chars().count());
            Json(json!({ "text": heard.text.trim(), "language": heard.language, "ms": ms })).into_response()
        }
        Err(Unavailable(reason)) => {
            (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "unavailable", "reason": reason }))).into_response()
        }
    }
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
fn samples_of(content_type: &str, body: &[u8]) -> Result<Vec<i16>, Refusal> {
    #[derive(serde::Deserialize)]
    struct Json {
        wav_b64: String,
    }
    let decoded;
    let wav = match content_type.starts_with("application/json") {
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
        calls: AtomicUsize,
        heard_samples: AtomicUsize,
    }

    impl Fake {
        fn saying(text: &str, language: &str) -> Self {
            let says = Ok(Heard { text: text.into(), language: language.into() });
            Fake { says, calls: AtomicUsize::new(0), heard_samples: AtomicUsize::new(0) }
        }
        fn deaf(reason: &str) -> Self {
            Fake { says: Err(Unavailable(reason.into())), ..Fake::saying("", "") }
        }
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl SttProvider for Fake {
        async fn hear(&self, samples: &[i16]) -> Result<Heard, Unavailable> {
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
        let response = answer(stt, (level, content_type), body).await;
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
        assert_eq!(body, json!({ "error": "unavailable", "reason": reason }));
    }

    /// The route is `/api/tts`'s class, and her door's: any device may reach
    /// it, the handler keeps it to the owner.
    #[test]
    fn the_route_is_classed_as_her_voice_is() {
        use crate::server::routes::classes::class_of;
        assert_eq!(class_of("POST", "/api/stt"), class_of("POST", "/api/tts"));
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

    /// The provider is handed the sound and nothing else: samples, no path or URL.
    #[test]
    fn a_hearing_backend_is_handed_samples_alone() {
        let code = include_str!("stt.rs");
        assert!(code.contains("async fn hear(&self, samples: &[i16]) -> Result<Heard, Unavailable>;"));
    }
}

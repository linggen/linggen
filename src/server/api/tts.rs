//! `POST /api/tts` — synthesize speech for Yinyue's voice.
//!
//! Voice synthesis sits behind the [`TtsProvider`] trait so the engine that
//! produces the audio is a swap, not a rewrite. The daemon's voice is
//! Qwen3-TTS ([`super::tts_mlx`]); [`SystemSayProvider`] is the one fallback
//! (see [`default_provider`]).
//!
//! A model-holding provider lives in
//! [`ServerState`](crate::server::ServerState) so it loads once, not per
//! request — `SystemSayProvider` is stateless and simply ignored by that rule.
//!
//! Returns one COMPLETE WAV clip, not a stream. A long line is cut into short
//! clauses ([`super::tts_text`]) — synthesized one by one and joined with
//! their pauses — because the GPU voice drifts on long lines. The web surface
//! streams by asking for each clause itself: the `pet_speak` event carries the
//! clauses, the surface plays the first while the next is made, and a
//! one-clause request here is just that clause.

use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use crate::server::ServerState;

#[derive(Deserialize)]
pub(crate) struct TtsRequest {
    pub text: String,
    /// Provider-specific voice id (e.g. a macOS voice name). Optional —
    /// providers fall back to their default voice.
    #[serde(default)]
    pub voice: Option<String>,
}

/// A voice backend: text in, WAV bytes (RIFF/PCM) out. One impl per engine.
#[async_trait]
pub trait TtsProvider: Send + Sync {
    /// Synthesize `text` to a complete WAV clip. `voice` is an optional
    /// provider-specific voice id; `None` uses the provider default.
    async fn synthesize(&self, text: &str, voice: Option<&str>) -> anyhow::Result<Vec<u8>>;

    /// Load heavy resources ahead of first use, off the hot path. Default
    /// no-op (stateless providers need nothing); a model-backed provider
    /// overrides this so the first user-facing utterance isn't slow.
    async fn prewarm(&self) {}

    /// Whether this provider's weights still need downloading. Default
    /// `false` (nothing to fetch); the boot task asks before it queues
    /// [`fetch`](TtsProvider::fetch) for a quiet moment.
    fn needs_fetch(&self) -> bool {
        false
    }

    /// Download the weights to disk without loading them. Called by the
    /// boot task when the machine is quiet, inside the runtime's download
    /// slot, pet on or off — loading stays with [`prewarm`](TtsProvider::prewarm).
    async fn fetch(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Whether to pre-warm this provider's model at daemon boot. A light model
    /// says `true` for a fast first utterance; a heavier one
    /// can say `false` to keep its download/load lazy. The boot caller also gates
    /// this on the pet being enabled.
    fn prewarm_on_boot(&self) -> bool {
        false
    }
}

/// The daemon's voice — the single switch point for the daemon's TTS.
/// Linggen Cloud, Qwen3-TTS on the GPU via the managed runtime's sidecar, or
/// `say`: the chain ends at the system voice, so the pet always has speech.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Source { Cloud, Qwen, System }

impl Source {
    /// The order of preference when the person has chosen nothing: Linggen
    /// Cloud's voice (Yinyue's, the same on the Mac and the phone) for a
    /// subscriber, then the Mac's own voices. Its readiness is a fact read
    /// with no network ([`Source::readiness`]), so a person who is not a
    /// subscriber, or is offline, is simply on the next one.
    pub(super) const ALL: [Self; 3] = [Self::Cloud, Self::Qwen, Self::System];

    pub(super) fn id(self) -> &'static str {
        match self { Self::Cloud => "linggen-cloud", Self::Qwen => "qwen", Self::System => "system" }
    }

    /// Whether the voice sounds like Yinyue herself (the GPU voice, or Linggen Cloud's Serena).
    pub(super) fn is_natural(self) -> bool { matches!(self, Self::Cloud | Self::Qwen) }

    /// Which voices can speak now. Linggen Cloud's is the owner's alone: it
    /// spends the owner's subscription and sends the text out, so a caller who
    /// is not the owner (`person::owners_only`, as `/api/live` asks) is never
    /// given it.
    pub(super) fn readiness(owner: bool) -> [bool; 3] {
        let system = std::env::var_os("PATH").is_some_and(|path| {
            std::env::split_paths(&path).any(|dir| dir.join("say").is_file())
        });
        let cloud = owner && crate::account::cloud_voice::speech_usable();
        [cloud, crate::runtime::voice_ready(), system]
    }
}

/// The one place the fallback chain is decided: the person's pick when it is
/// ready, else the first ready voice in [`Source::ALL`] order (so a stored
/// pick that is no longer a voice, e.g. "kokoro", reads as no choice).
pub(super) fn default_provider(preference: Option<&str>, ready: [bool; 3]) -> Option<Source> {
    let usable = || Source::ALL.into_iter().zip(ready).filter_map(|(source, ready)| ready.then_some(source));
    usable().find(|source| Some(source.id()) == preference).or_else(|| usable().next())
}

/// The system voice this text needs: `say` does not switch language by
/// script, so any Han character means a Mandarin voice. "Tingting" is the
/// unambiguous zh_CN name (several others, e.g. "Eddy", exist under both
/// zh_CN and zh_TW). `None` leaves the system's default voice.
fn say_voice_for(text: &str) -> Option<&'static str> {
    (super::tts_text::han_count(text) > 0).then_some("Tingting")
}

/// macOS `say` — neural system voices, no dependencies. The fallback:
/// reached when Qwen3-TTS isn't ready or fails, so the pet always has a voice.
pub struct SystemSayProvider;

#[async_trait]
impl TtsProvider for SystemSayProvider {
    async fn synthesize(&self, text: &str, voice: Option<&str>) -> anyhow::Result<Vec<u8>> {
        let voice = voice.or_else(|| say_voice_for(text));
        // Write the text to a temp file and feed it via `-f` so the spoken
        // string can't be mis-parsed as flags and isn't length-limited.
        let dir = tempfile::tempdir()?;
        let in_path = dir.path().join("say-in.txt");
        let out_path = dir.path().join("say-out.wav");
        tokio::fs::write(&in_path, text).await?;

        let mut cmd = tokio::process::Command::new("say");
        cmd.arg("-f")
            .arg(&in_path)
            .arg("-o")
            .arg(&out_path)
            // 16-bit PCM WAVE @ 22.05kHz — what Web Audio decodeAudioData wants.
            .arg("--file-format=WAVE")
            .arg("--data-format=LEI16@22050");
        if let Some(v) = voice {
            cmd.arg("-v").arg(v);
        }

        let status = cmd.status().await?;
        if !status.success() {
            anyhow::bail!("`say` exited with {status}");
        }
        Ok(tokio::fs::read(&out_path).await?)
    }
}

/// The voices a line may name: the GPU voice's speakers.
fn is_listed_voice(name: &str) -> bool {
    super::tts_mlx::SPEAKERS.contains(&name)
}

/// The whole line as one clip: its spoken copy, clause by clause, each
/// followed by its pause. A one-clause line goes straight through. A voice
/// that is not one of the listed voices, by name, is not passed on: the
/// line speaks in the default voice.
async fn speak_line(
    tts: &dyn TtsProvider,
    text: &str,
    voice: Option<&str>,
) -> anyhow::Result<Vec<u8>> {
    let started = std::time::Instant::now();
    let voice = voice.filter(|v| is_listed_voice(v));
    let chunks = super::tts_text::chunks(text);
    if let [only] = chunks.as_slice() {
        return tts.synthesize(&only.text, voice).await;
    }
    let mut parts = Vec::with_capacity(chunks.len());
    for chunk in &chunks {
        let wav = tts.synthesize(&chunk.text, voice).await?;
        parts.push((super::tts_wav::decode(&wav)?, chunk.pause_ms));
    }
    tracing::info!(
        "[tts] {} clauses in {} ms",
        chunks.len(),
        started.elapsed().as_millis()
    );
    Ok(super::tts_wav::encode(&super::tts_wav::join(&parts)))
}

/// The line as ONE request, for a voice that does not drift on a long line
/// (Linggen Cloud's): one round trip instead of one per clause. A line over
/// the service's 600 characters goes clause by clause, as the others do.
async fn speak_whole_line(tts: &dyn TtsProvider, text: &str) -> anyhow::Result<Vec<u8>> {
    let spoken = super::tts_text::spoken_copy(text);
    if spoken.trim().is_empty() || spoken.chars().count() > CLOUD_LINE_MAX_CHARS {
        return speak_line(tts, text, None).await;
    }
    tts.synthesize(&spoken, None).await
}

/// What one Linggen Cloud read-aloud request may hold (the site refuses more).
const CLOUD_LINE_MAX_CHARS: usize = 600;

/// Whether a clip made now is made without fetching anything, and in which
/// voice: the one judgement behind `GET /api/tts`. What a page asks before it
/// turns "read her replies aloud" on. `natural`: the GPU voice (Qwen3-TTS) is on
/// disk, so lines are spoken in her own voice; without it a line is the system's
/// `say`. `speaks` is false only on a machine with none of the voices.
#[derive(Debug, PartialEq)]
pub(crate) struct Speech {
    pub speaks: bool,
    pub natural: bool,
}

/// Why there is no voice: its fetch gave up (and why), or it is not here yet.
fn no_voice_reason() -> String {
    let again = "It tries again on its own.";
    match crate::runtime::voice_note() {
        Some(note) => note.told(crate::runtime::what_is_fetched(crate::runtime::TTS_MODEL), again),
        None => "This Mac has no voice to read with.".to_string(),
    }
}

/// Whether the caller is the owner: the rule the settings routes ask (`providers::owner_refusal`).
fn is_owner(headers: &HeaderMap) -> bool {
    crate::server::person::of_headers(headers).is_owner()
}

fn speech_now(pref: Option<&str>, owner: bool) -> Speech {
    let source = default_provider(pref, Source::readiness(owner));
    Speech { speaks: source.is_some(), natural: source.is_some_and(Source::is_natural) }
}

/// GET /api/tts — can a line be spoken now, and in her own voice? Reads the
/// disk and starts nothing: no fetch, no model load, no synthesis.
pub(crate) async fn tts_status_handler(State(state): State<Arc<ServerState>>, headers: HeaderMap) -> impl IntoResponse {
    let config = state.manager.get_config_snapshot().await;
    let now = speech_now(config.uses.speak.as_deref(), is_owner(&headers));
    match now.speaks {
        true => Json(serde_json::json!({ "speaks": true, "natural": now.natural })),
        false => Json(serde_json::json!({
            "speaks": false,
            "natural": false,
            "reason": no_voice_reason()
        })),
    }
}

/// Linggen Cloud's voice as a [`TtsProvider`]: each clip asks linggen.dev
/// (`account::cloud_voice`), which reads it in Yinyue's voice and counts the
/// characters against the subscription. The voice is the service's (Serena),
/// so a voice the caller names is not passed on.
struct CloudProvider;

#[async_trait]
impl TtsProvider for CloudProvider {
    async fn synthesize(&self, text: &str, _voice: Option<&str>) -> anyhow::Result<Vec<u8>> {
        crate::account::cloud_voice::speak(text)
            .await
            .map_err(|why| anyhow::anyhow!("Linggen Cloud voice: {why:?}"))
    }
}

/// POST /api/tts — `{ text, voice? }` → `audio/wav` bytes.
pub(crate) async fn tts_handler(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    Json(req): Json<TtsRequest>,
) -> impl IntoResponse {
    if req.text.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "empty text").into_response();
    }

    // No voice named → the configured pet voice, applied per clip so a
    // Settings change speaks on the very next line.
    let config = state.manager.get_config_snapshot().await;
    let owner = is_owner(&headers);
    let mut failure = None;
    // Linggen Cloud may fail mid-line (cap, offline); a refusal cools it off
    // (`cloud_voice`), so the second pass finds the next voice ready.
    for _ in 0..2 {
        let source = default_provider(config.uses.speak.as_deref(), Source::readiness(owner));
        let (tts, voice): (&dyn TtsProvider, Option<&str>) = match source {
            Some(Source::Cloud) => (&CloudProvider, None),
            Some(Source::Qwen) => (state.tts.as_ref(), req.voice.as_deref().or(Some(&config.pet.voice))),
            Some(Source::System) => (&SystemSayProvider, None),
            None => return (StatusCode::SERVICE_UNAVAILABLE, "No voice is ready.").into_response(),
        };
        let line = match source {
            Some(Source::Cloud) => speak_whole_line(tts, &req.text).await,
            _ => speak_line(tts, &req.text, voice).await,
        };
        match line {
            Ok(wav) => return (StatusCode::OK, [(header::CONTENT_TYPE, "audio/wav")], wav).into_response(),
            Err(e) => {
                tracing::warn!("[tts] synthesis failed: {e}");
                failure = Some(e);
                if source != Some(Source::Cloud) {
                    break;
                }
            }
        }
    }
    let e = failure.expect("a loop that did not return failed once");
    (StatusCode::INTERNAL_SERVER_ERROR, format!("tts synthesis failed: {e}")).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records the voice each clip was asked for; answers no audio.
    struct Heard(std::sync::Mutex<Vec<Option<String>>>);

    #[async_trait]
    impl TtsProvider for Heard {
        async fn synthesize(&self, _text: &str, voice: Option<&str>) -> anyhow::Result<Vec<u8>> {
            self.0.lock().unwrap().push(voice.map(str::to_string));
            Ok(Vec::new())
        }
    }

    /// A voice reaches a provider only when it is one of the listed voices,
    /// by name — never a path or a name to fetch.
    #[tokio::test]
    async fn only_a_listed_voice_reaches_the_provider() {
        let heard = Heard(std::sync::Mutex::new(Vec::new()));
        for voice in ["../../somewhere/x", "vivian", "af_heart", "other/repo"] {
            let _ = speak_line(&heard, "Hello.", Some(voice)).await;
        }
        let heard = heard.0.into_inner().unwrap();
        let want = [None, Some("vivian".to_string()), None, None];
        assert_eq!(heard, want);
    }

    /// The status says "speaks" when any of her voices is there, "natural"
    /// only for the GPU voice, and never starts anything: it is a function of
    /// three facts read from disk.
    #[test]
    fn speech_prefers_a_ready_choice_then_the_builtin_order() {
        let ready = [true; 3];
        // Cloud (Yinyue's own voice for a subscriber) is the default; a person's pick wins.
        assert_eq!(default_provider(None, ready), Some(Source::Cloud));
        assert_eq!(default_provider(Some("qwen"), ready), Some(Source::Qwen));
        // A stored "kokoro" (a removed voice) reads as no choice.
        assert_eq!(default_provider(Some("kokoro"), ready), Some(Source::Cloud));
        assert_eq!(default_provider(Some("kokoro"), [false, true, true]), Some(Source::Qwen));
        assert_eq!(default_provider(Some("system"), ready), Some(Source::System));
        // Not a subscriber, offline, or cooling off: the Mac's own voices, as before.
        assert_eq!(default_provider(None, [false, true, true]), Some(Source::Qwen));
        assert_eq!(default_provider(Some("linggen-cloud"), [false, true, true]), Some(Source::Qwen));
        // Qwen3-TTS not ready: the system voice.
        assert_eq!(default_provider(Some("qwen"), [false, false, true]), Some(Source::System));
        assert_eq!(default_provider(None, [false; 3]), None);
        assert!(Source::Cloud.is_natural() && Source::Qwen.is_natural() && !Source::System.is_natural());
    }

    /// Linggen Cloud's voice is the owner's alone: it spends the owner's subscription and sends the text out.
    #[test]
    fn linggen_clouds_voice_is_never_ready_for_a_caller_who_is_not_the_owner() {
        assert!(!Source::readiness(false)[0], "Cloud is the first voice in the order");
        assert_eq!(Source::ALL[0], Source::Cloud);
    }

    /// Lock: the status route reads, it does not make or fetch. Its handler and
    /// the judgement behind it name no synthesis, no fetch and no load.
    #[test]
    fn the_status_route_starts_nothing() {
        let code = include_str!("tts.rs");
        let production = &code[..code.find("#[cfg(test)]").unwrap()];
        let from = production.find("fn speech_now").unwrap();
        let to = production.find("/// POST /api/tts").unwrap();
        let status = &production[from..to];
        for starts in [".synthesize(", ".fetch(", ".prewarm(", "speak_line", "get_bytes"] {
            assert!(!status.contains(starts), "the status route calls `{starts}`");
        }
    }

    /// Lock: the removed voice is named nowhere in the code but here, where a
    /// stored "kokoro" is read as no choice (`default_provider`, its test).
    #[test]
    fn the_removed_voice_is_named_nowhere_else() {
        fn scan(dir: &std::path::Path, hits: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    scan(&path, hits);
                    continue;
                }
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if !["rs", "ts", "tsx", "js", "css", "toml", "json"].contains(&ext) || path.ends_with("api/tts.rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                if text.to_lowercase().contains("kokoro") {
                    hits.push(path.display().to_string());
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut hits = Vec::new();
        for dir in ["crates", "src", "ui/src"] {
            scan(&root.join(dir), &mut hits);
        }
        assert!(hits.is_empty(), "Kokoro was removed, yet named in: {hits:?}");
    }

    #[test]
    fn say_speaks_han_text_in_a_mandarin_voice() {
        // `say`'s default English voice reads Han characters as gibberish.
        assert_eq!(say_voice_for("师父，我在这里。"), Some("Tingting"));
        // One Han word makes the line Chinese.
        assert_eq!(say_voice_for("Master，我在这里"), Some("Tingting"));
        assert_eq!(say_voice_for("Master, I am here."), None);
        assert_eq!(say_voice_for("Hello ——"), None);
    }
}

//! `POST /api/tts` — synthesize speech for Yinyue's voice.
//!
//! Voice synthesis sits behind the [`TtsProvider`] trait so the engine that
//! produces the audio is a swap, not a rewrite. The daemon's voice is
//! [`KokoroProvider`] (see [`default_provider`]); [`SystemSayProvider`] is no
//! longer a stand-in for it but its offline fallback.
//!
//! A model-holding provider lives in
//! [`ServerState`](crate::server::ServerState) so it loads once, not per
//! request — `SystemSayProvider` is stateless and simply ignored by that rule.
//!
//! Returns one COMPLETE WAV clip, not a stream. The caller needs the whole
//! buffer anyway: the web surface decodes it and computes an amplitude
//! envelope from the decoded samples to drive lip-sync, which cannot be done
//! incrementally. Chunked synthesis would mean an incremental envelope too.

use std::sync::Arc;

use any_tts::{ModelType, SynthesisRequest, TtsConfig, TtsModel};
use async_trait::async_trait;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use tokio::sync::OnceCell;

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

    /// Whether to pre-warm this provider's model at daemon boot. A light model
    /// (Kokoro, ~300 MB) says `true` for a fast first utterance; a heavier one
    /// can say `false` to keep its download/load lazy. The boot caller also gates
    /// this on the pet being enabled.
    fn prewarm_on_boot(&self) -> bool {
        false
    }
}

/// The daemon's voice: official Kokoro — fast (~5x realtime on Metal, sub-2s)
/// with its preset voices. The single switch point for the daemon's TTS.
pub fn default_provider() -> Arc<dyn TtsProvider> {
    Arc::new(KokoroProvider::new())
}

/// Kokoro's own voice packs carry the LANGUAGE, not just the timbre: its
/// phonemizer reads the language off the voice id's first letter ('a' =
/// American English, 'z' = Mandarin). So picking the voice IS picking the
/// language, and an English voice handed Chinese text does not error — it
/// emits about half a second of noise. Hence [`voice_for`].
const VOICE_EN: &str = "af_heart"; // warm American female
const VOICE_ZH: &str = "zf_xiaoxiao"; // Mandarin female

/// The voice this text needs. Any Han character means Mandarin: Kokoro cannot
/// mix scripts inside one utterance, so a mixed line follows its Han content
/// (she is likelier to be answering in Chinese than naming one English word).
fn voice_for(text: &str) -> &'static str {
    let has_han = text.chars().any(|c| {
        matches!(c as u32,
            0x4E00..=0x9FFF   // CJK Unified Ideographs
            | 0x3400..=0x4DBF // Extension A
            | 0xF900..=0xFAFF // Compatibility Ideographs
        )
    });
    if has_han {
        VOICE_ZH
    } else {
        VOICE_EN
    }
}

/// macOS `say` — neural system voices, no dependencies. Kokoro's fallback:
/// reached when the weights can't be loaded (offline, download failed), so the
/// pet always has a voice. Not the daemon's default provider.
pub struct SystemSayProvider;

#[async_trait]
impl TtsProvider for SystemSayProvider {
    async fn synthesize(&self, text: &str, voice: Option<&str>) -> anyhow::Result<Vec<u8>> {
        // `say` does not switch language by script either: its default English
        // voice reads Han characters as gibberish. Same rule as Kokoro — the
        // script picks the voice. "Tingting" is the unambiguous zh_CN name
        // (several others, e.g. "Eddy", exist under both zh_CN and zh_TW).
        let voice = match voice {
            Some(v) => Some(v),
            None if voice_for(text) == VOICE_ZH => Some("Tingting"),
            None => None,
        };
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

/// Kokoro-82M (StyleTTS2) running locally via candle (the `any-tts` crate) —
/// the production voice. Pure-Rust phonemizer (no espeak-ng) and a candle
/// backend (no ONNX Runtime), so there's nothing native to bundle.
///
/// The ~300 MB weights are fetched from HuggingFace (`hexgrad/Kokoro-82M`) on
/// first use and cached under `~/.linggen/models`. Loading is lazy by
/// construction (a `OnceCell`), but [`prewarm_on_boot`](TtsProvider::prewarm_on_boot)
/// returns `true`, so when the pet is enabled the daemon warms the model at
/// boot rather than making Yinyue's first utterance pay for it; with the pet
/// disabled nothing loads until she actually speaks. If it can't load
/// (offline, download failed), synthesis falls back to `say` so the pet always
/// has a voice.
pub struct KokoroProvider {
    model: OnceCell<Arc<dyn TtsModel>>,
    fallback: SystemSayProvider,
}

impl KokoroProvider {
    pub fn new() -> Self {
        Self {
            model: OnceCell::new(),
            fallback: SystemSayProvider,
        }
    }

    /// Voice packs live in a directory we own, not wherever the HuggingFace
    /// cache happens to put them: the crate only auto-discovers voices already
    /// on disk and has no per-voice download, so owning the path is what lets
    /// us guarantee a voice is there before asking for it.
    fn voices_dir() -> std::path::PathBuf {
        crate::util::resolve_path(std::path::Path::new("~/.linggen/models/kokoro-voices"))
    }

    /// Make sure one voice pack (~500 KB) is on disk, fetching it if not.
    /// Best-effort by design — the caller falls back to `say` if this fails,
    /// so a missing voice costs timbre, never speech.
    async fn ensure_voice(name: &str) -> anyhow::Result<()> {
        let dir = Self::voices_dir();
        let path = dir.join(format!("{name}.pt"));
        if path.exists() {
            return Ok(());
        }
        tokio::fs::create_dir_all(&dir).await?;
        let url =
            format!("https://huggingface.co/hexgrad/Kokoro-82M/resolve/main/voices/{name}.pt");
        tracing::info!("[tts] fetching voice pack {name}");
        let bytes = reqwest::get(&url).await?.error_for_status()?.bytes().await?;
        // Write via a temp file and rename: a half-written .pt would be
        // indistinguishable from a good one on the next boot.
        let tmp = dir.join(format!(".{name}.pt.part"));
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }

    /// Load the model once, on first use. Download + candle init are blocking,
    /// so they run on a blocking thread; `OnceCell` serializes concurrent
    /// first-callers behind a single init.
    async fn model(&self) -> anyhow::Result<Arc<dyn TtsModel>> {
        self.model
            .get_or_try_init(|| async {
                let model = tokio::task::spawn_blocking(|| -> anyhow::Result<Arc<dyn TtsModel>> {
                    // Keep all model state under ~/.linggen (project convention).
                    if std::env::var_os("HF_HUB_CACHE").is_none() {
                        let cache = crate::util::resolve_path(std::path::Path::new(
                            "~/.linggen/models/hf-hub",
                        ));
                        std::env::set_var("HF_HUB_CACHE", cache);
                    }
                    // No path set → any-tts downloads hexgrad/Kokoro-82M from HF.
                    // Voices come from our own directory (see `voices_dir`).
                    let model = any_tts::load_model(
                        TtsConfig::new(ModelType::Kokoro).with_voices_dir(Self::voices_dir()),
                    )?;
                    Ok(Arc::from(model))
                })
                .await??;
                Ok::<_, anyhow::Error>(model)
            })
            .await
            .cloned()
    }
}

#[async_trait]
impl TtsProvider for KokoroProvider {
    async fn synthesize(&self, text: &str, voice: Option<&str>) -> anyhow::Result<Vec<u8>> {
        // The caller may name a voice; otherwise the script chooses one, which
        // is also how the language gets chosen.
        let voice = voice.unwrap_or_else(|| voice_for(text)).to_string();

        // The crate reads the pack from `voices_dir` on every synthesis, so a
        // voice fetched now is usable now — no restart, no model reload.
        let voice_ready = Self::ensure_voice(&voice).await;
        let model = match (&voice_ready, self.model().await) {
            (Ok(()), Ok(m)) => m,
            (voice_err, model_res) => {
                if let Err(e) = voice_err {
                    tracing::warn!("[tts] voice {voice} unavailable ({e}); using `say`");
                }
                if let Err(e) = model_res {
                    tracing::warn!("[tts] Kokoro unavailable ({e}); using `say` for this clip");
                }
                // `say` doesn't share Kokoro's voice ids — let it pick by script.
                return self.fallback.synthesize(text, None).await;
            }
        };

        let text = text.to_string();
        // candle inference is blocking and CPU/Metal-bound → off the async pool.
        let wav = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
            let req = SynthesisRequest::new(text).with_voice(voice);
            Ok(model.synthesize(&req)?.get_wav())
        })
        .await??;
        Ok(wav)
    }

    async fn prewarm(&self) {
        // Both voices up front (~500 KB each), so neither language's first line
        // pays for a download or falls back to `say`.
        for v in [VOICE_EN, VOICE_ZH] {
            if let Err(e) = Self::ensure_voice(v).await {
                tracing::warn!("[tts] could not fetch voice {v} ({e})");
            }
        }
        // Loading the ~300 MB weights onto the GPU takes several seconds; do it
        // now so Yinyue's first utterance is as fast as the rest.
        match self.model().await {
            Ok(_) => tracing::info!("[tts] Kokoro model pre-warmed"),
            Err(e) => tracing::warn!("[tts] Kokoro pre-warm failed ({e}); will fall back to `say`"),
        }
    }

    /// Light (~300 MB) — safe to load at boot for a snappy first utterance.
    fn prewarm_on_boot(&self) -> bool {
        true
    }
}

/// POST /api/tts — `{ text, voice? }` → `audio/wav` bytes.
pub(crate) async fn tts_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<TtsRequest>,
) -> impl IntoResponse {
    if req.text.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "empty text").into_response();
    }

    match state.tts.synthesize(&req.text, req.voice.as_deref()).await {
        Ok(wav) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "audio/wav")],
            wav,
        )
            .into_response(),
        Err(e) => {
            tracing::warn!("[tts] synthesis failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("tts synthesis failed: {e}"),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_text_takes_the_english_voice() {
        assert_eq!(voice_for("Master, I am here."), VOICE_EN);
        assert_eq!(voice_for(""), VOICE_EN);
        // Latin text with CJK punctuation only is still English.
        assert_eq!(voice_for("Hello ——"), VOICE_EN);
    }

    #[test]
    fn han_text_takes_the_mandarin_voice() {
        // The bug this guards: an English voice on Han text does not error, it
        // emits ~0.5s of noise, so nothing downstream notices.
        assert_eq!(voice_for("师父，我在这里。"), VOICE_ZH);
        assert_eq!(voice_for("你好"), VOICE_ZH);
    }

    #[test]
    fn a_mixed_line_follows_its_han_content() {
        // Kokoro cannot switch script mid-utterance; one Han word means the
        // line is likelier Chinese than an English sentence naming a词.
        assert_eq!(voice_for("Master，我在这里"), VOICE_ZH);
    }

    #[test]
    fn the_voice_prefix_is_what_selects_the_language() {
        // Guards the invariant the whole fix rests on: any-tts reads the
        // language off the first letter ('a' = en-US, 'z' = zh).
        assert!(VOICE_EN.starts_with('a'));
        assert!(VOICE_ZH.starts_with('z'));
    }
}

//! `POST /api/tts` — synthesize speech for Yinyue's voice.
//!
//! Voice synthesis sits behind the [`TtsProvider`] trait so the engine that
//! produces the audio is a swap, not a rewrite. Phase 1 ships
//! [`SystemSayProvider`] (macOS `say` — neural system voices, zero
//! dependencies), which proves the full pipeline end-to-end: text in, WAV
//! bytes out, played by the desktop pet with amplitude-driven lip-sync.
//!
//! The planned local provider is Kokoro (ONNX, bundled, cross-platform); it
//! drops in behind this same trait. A model-holding provider lives in
//! [`ServerState`](crate::server::ServerState) so it loads once, not per
//! request — `SystemSayProvider` is stateless and simply ignored by that rule.

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

    /// Whether to pre-warm this provider's model at daemon boot. Light models
    /// (Kokoro, ~300 MB) say `true` for a fast first utterance; heavy ones
    /// (Qwen3, ~2 GB download) say `false` so the download is **lazy** — pulled
    /// on first speak, never forced on a fresh install. The boot caller also
    /// gates this on the pet being enabled.
    fn prewarm_on_boot(&self) -> bool {
        false
    }
}

/// Total physical RAM in GB, or `0.0` if it can't be determined (callers treat
/// unknown as "small" — the conservative pick that avoids a 2 GB download).
fn total_ram_gb() -> f64 {
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
        {
            if let Ok(s) = String::from_utf8(out.stdout) {
                if let Ok(bytes) = s.trim().parse::<u64>() {
                    return bytes as f64 / 1_073_741_824.0;
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string("/proc/meminfo") {
            for line in s.lines() {
                if let Some(rest) = line.strip_prefix("MemTotal:") {
                    if let Some(kb) = rest
                        .split_whitespace()
                        .next()
                        .and_then(|v| v.parse::<u64>().ok())
                    {
                        return kb as f64 / 1_048_576.0;
                    }
                }
            }
        }
    }
    0.0
}

/// RAM above which `auto` picks the heavier, better-sounding Qwen3 voice.
const RAM_QWEN_THRESHOLD_GB: f64 = 16.0;

/// Build the daemon's TTS provider from the configured engine
/// (`agent.pet.voice_engine`). `"auto"` decides by machine RAM: Qwen3-TTS
/// VoiceDesign (her designed voice) on >16 GB, Kokoro (light preset) otherwise.
/// The Qwen provider keeps Kokoro→`say` as its own fallback, so the pet is never
/// mute. The single switch point for the daemon's voice.
pub fn make_tts_provider(engine: &str) -> Arc<dyn TtsProvider> {
    let choice = match engine.trim().to_ascii_lowercase().as_str() {
        "qwen" | "qwen3" => "qwen",
        "kokoro" => "kokoro",
        _ => {
            let gb = total_ram_gb();
            let pick = if gb > RAM_QWEN_THRESHOLD_GB { "qwen" } else { "kokoro" };
            tracing::info!(
                "[tts] auto-select voice engine: {:.1} GB RAM → {pick}",
                gb
            );
            pick
        }
    };
    match choice {
        "qwen" => Arc::new(Qwen3VoiceDesignProvider::new()),
        _ => Arc::new(KokoroProvider::new()),
    }
}

/// macOS `say` — neural system voices, no dependencies. A faithful stand-in
/// that exercises the entire voice path while the bundled Kokoro provider is
/// built out behind the same trait.
pub struct SystemSayProvider;

#[async_trait]
impl TtsProvider for SystemSayProvider {
    async fn synthesize(&self, text: &str, voice: Option<&str>) -> anyhow::Result<Vec<u8>> {
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
/// first use and cached under `~/.linggen/models`. Loading is therefore lazy:
/// the daemon boots instantly and the model only materializes when Yinyue
/// first speaks. If it can't load (offline, download failed), synthesis falls
/// back to `say` so the pet always has a voice.
pub struct KokoroProvider {
    model: OnceCell<Arc<dyn TtsModel>>,
    voice: String,
    fallback: SystemSayProvider,
}

impl KokoroProvider {
    pub fn new() -> Self {
        Self {
            model: OnceCell::new(),
            voice: "af_heart".to_string(), // warm American-female default
            fallback: SystemSayProvider,
        }
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
                    let model = any_tts::load_model(TtsConfig::new(ModelType::Kokoro))?;
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
        let model = match self.model().await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("[tts] Kokoro unavailable ({e}); using `say` for this clip");
                // `say` doesn't share Kokoro's voice ids — use its default voice.
                return self.fallback.synthesize(text, None).await;
            }
        };

        let voice = voice.unwrap_or(&self.voice).to_string();
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

/// Yinyue's voice, described in words. Qwen3-TTS VoiceDesign conditions the
/// voice on this string — tuning her sound = editing this text (iterate by ear;
/// there are no presets to hunt). Kept terse and concrete: age, brightness,
/// warmth, pace, demeanor.
const YINYUE_VOICE_DESCRIPTION: &str = "A young woman's voice, late teens: bright \
and clear, gently warm with a touch of playfulness, soft-spoken and unhurried, \
composed rather than bubbly.";

/// HuggingFace repo for the VoiceDesign checkpoint (Apache-2.0, ~1.9 GB BF16).
/// The codec tokenizer is pulled from `Qwen/Qwen3-TTS-Tokenizer-12Hz` by the
/// crate automatically.
const QWEN3_VOICEDESIGN_REPO: &str = "Qwen/Qwen3-TTS-12Hz-1.7B-VoiceDesign";

/// Qwen3-TTS **VoiceDesign** — a 1.7B autoregressive TTS whose voice is set by a
/// NATURAL-LANGUAGE DESCRIPTION (`instruct`) rather than a fixed preset id. This
/// is how Yinyue gets a young, characterful voice that Kokoro's preset catalog
/// can't provide. It's heavier than Kokoro (~1.9 GB BF16 weights + a separate
/// codec tokenizer, and autoregressive so slower per utterance), so it loads
/// lazily and falls back to Kokoro — which itself falls back to `say` — if it
/// can't load or synthesize. The pet therefore always has a voice.
pub struct Qwen3VoiceDesignProvider {
    inner: Arc<Qwen3Inner>,
    fallback: KokoroProvider,
}

/// Shared, background-loadable state for the Qwen3 model. Held behind an `Arc`
/// so a `synthesize` call can spawn the heavy lazy load without blocking — the
/// `OnceCell` makes concurrent triggers converge on a single download + init.
struct Qwen3Inner {
    model: OnceCell<Arc<dyn TtsModel>>,
    description: String,
}

impl Qwen3Inner {
    /// Load the model once, on first use (download + candle init are blocking).
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
                    let cfg = TtsConfig::new(ModelType::Qwen3Tts)
                        .with_hf_model_id(QWEN3_VOICEDESIGN_REPO);
                    let model = any_tts::load_model(cfg)?;
                    Ok(Arc::from(model))
                })
                .await??;
                Ok::<_, anyhow::Error>(model)
            })
            .await
            .cloned()
    }
}

impl Qwen3VoiceDesignProvider {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Qwen3Inner {
                model: OnceCell::new(),
                description: YINYUE_VOICE_DESCRIPTION.to_string(),
            }),
            fallback: KokoroProvider::new(),
        }
    }
}

#[async_trait]
impl TtsProvider for Qwen3VoiceDesignProvider {
    async fn synthesize(&self, text: &str, voice: Option<&str>) -> anyhow::Result<Vec<u8>> {
        // VoiceDesign has no named speakers — the voice IS the description; any
        // caller-supplied `voice` id is meaningless here and ignored.
        let _ = voice;

        // If the heavy Qwen model is already resident, speak in her designed
        // voice. If not, DON'T block this line on the ~2 GB download: trigger the
        // load in the background (once) and serve the light Kokoro bridge now.
        // The next line after Qwen finishes loading upgrades to it seamlessly.
        let Some(model) = self.inner.model.get().cloned() else {
            let inner = self.inner.clone();
            tokio::spawn(async move {
                match inner.model().await {
                    Ok(_) => tracing::info!(
                        "[tts] Qwen3 VoiceDesign ready — future lines use her designed voice"
                    ),
                    Err(e) => tracing::warn!(
                        "[tts] Qwen3 background load failed ({e}); staying on Kokoro"
                    ),
                }
            });
            tracing::info!("[tts] Qwen3 still loading — Kokoro bridges this line");
            return self.fallback.synthesize(text, None).await;
        };

        let text_owned = text.to_string();
        let description = self.inner.description.clone();
        // candle inference is blocking and CPU/Metal-bound → off the async pool.
        let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
            let req = SynthesisRequest::new(text_owned).with_instruct(description);
            Ok(model.synthesize(&req)?.get_wav())
        })
        .await?;

        match result {
            Ok(wav) => Ok(wav),
            Err(e) => {
                tracing::warn!("[tts] Qwen3 synth failed ({e}); using Kokoro for this line");
                self.fallback.synthesize(text, None).await
            }
        }
    }

    async fn prewarm(&self) {
        // Warm only the LIGHT Kokoro bridge (~300 MB) so her very first line is
        // instant. The heavy Qwen model stays lazy — it downloads in the
        // background on her first speak (see `synthesize`), never at boot.
        self.fallback.prewarm().await;
    }

    /// True — but `prewarm` warms only the light Kokoro bridge, not Qwen's
    /// ~2 GB model (which stays lazy). Boot stays cheap, first line still instant.
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

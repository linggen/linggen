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

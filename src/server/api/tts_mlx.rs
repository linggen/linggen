//! Qwen3-TTS on the Mac's GPU, spoken through the resident Python sidecar
//! ([`super::mlx_sidecar`], shared with her hearing).
//!
//! One `{text, voice, lang}` request in — one short clause, see
//! [`super::tts_text`] — one complete base64 WAV out. A take that drifts
//! (too slow for its length) is made once more.
//!
//! Every failure — venv missing (Intel Mac, prewarm not done), sidecar
//! dead, malformed reply, timeout — falls back to the system voice
//! ([`SystemSayProvider`]), so the pet always has a voice. While the sidecar
//! is still loading, a line waits for it a bounded few seconds (the sidecar's
//! slot wait) and then goes to the system voice.

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;

use super::mlx_sidecar::VoiceSidecar;
use super::tts::{SystemSayProvider, TtsProvider};
use super::tts_text::{self, Take};
use super::tts_wav;

/// Multilingual (zh + en in one timbre), so no script→voice
/// mapping is needed. Names must match the model's speaker roster.
const DEFAULT_VOICE: &str = "vivian";
pub(super) const SPEAKERS: [&str; 9] = [
    "serena", "vivian", "uncle_fu", "ryan", "aiden", "ono_anna", "sohee", "eric", "dylan",
];

/// Per-clip budget, scaled to the text: healthy synthesis runs ~0.3×
/// realtime, but the GPU can degrade machine-wide (thermal / competing
/// Metal work — seen live 2026-08-19 at ~3× realtime), and a fixed 60s
/// bound meant a minute of silence before the system voice spoke. Scaling by length
/// keeps the wait proportional: give up early on a short line, allow a
/// long one its honest time.
fn synth_timeout(text: &str) -> std::time::Duration {
    let chars = text.chars().count() as u64;
    std::time::Duration::from_secs((12 + chars / 3).clamp(15, 60))
}

pub struct MlxTtsProvider {
    sidecar: Arc<VoiceSidecar>,
}

impl MlxTtsProvider {
    pub fn new(sidecar: Arc<VoiceSidecar>) -> Self {
        Self { sidecar }
    }

    /// Venv and model both on disk: a sidecar spawned before the model
    /// stage lands would download it in front of the first line.
    fn available() -> bool {
        crate::runtime::voice_ready()
    }

    /// One clip from the sidecar. Any error — busy, dead, timed out — is
    /// the caller's cue to fall back.
    async fn request(&self, text: &str, voice: &str) -> anyhow::Result<Vec<u8>> {
        let req = serde_json::json!({
            "text": text,
            "voice": voice,
            "lang": tts_text::language_for(text, crate::language::setting().as_deref()),
        });
        let reply = self.sidecar.ask(&req, synth_timeout(text)).await?;
        let b64 = reply
            .get("wav_b64")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("reply missing wav_b64"))?;
        Ok(base64::engine::general_purpose::STANDARD.decode(b64)?)
    }
}

impl MlxTtsProvider {
    /// One clause, checked for drift: a take outside the healthy speech
    /// rate is made once more, and the closer of the two ships. Never
    /// the system voice on rate grounds — a slow Qwen take still says the
    /// right words (the clip Hanli heard as garbled transcribed at 3.8% CER).
    async fn checked(&self, text: &str, voice: &str) -> anyhow::Result<Vec<u8>> {
        let first = self.request(text, voice).await?;
        let verdict = judge_take(text, &first);
        if verdict.passes() {
            return Ok(first);
        }
        tracing::info!("[tts-mlx] take drifted ({verdict:?}), making it again: {text}");
        let Ok(second) = self.request(text, voice).await else {
            return Ok(first);
        };
        let again = judge_take(text, &second);
        tracing::info!("[tts-mlx] second take {again:?}");
        Ok(if again.shortfall() < verdict.shortfall() {
            second
        } else {
            first
        })
    }
}

/// The rate verdict on one take; an undecodable clip goes unjudged.
fn judge_take(text: &str, wav: &[u8]) -> Take {
    tts_wav::decode(wav).map_or(Take::Unjudged, |pcm| {
        tts_text::judge(text, tts_wav::voiced_secs(&pcm))
    })
}

#[async_trait]
impl TtsProvider for MlxTtsProvider {
    async fn synthesize(&self, text: &str, voice: Option<&str>) -> anyhow::Result<Vec<u8>> {
        if !Self::available() {
            // Intel Mac, or the runtime prewarm hasn't landed yet.
            return SystemSayProvider.synthesize(text, None).await;
        }
        // A voice that is not one of the speakers means the caller wasn't talking to us.
        let voice = match voice {
            Some(v) if SPEAKERS.contains(&v) => v,
            _ => DEFAULT_VOICE,
        };
        match self.checked(text, voice).await {
            Ok(wav) => Ok(wav),
            Err(e) => {
                tracing::warn!("[tts-mlx] {e:#}; falling back to the system voice for this clip");
                SystemSayProvider.synthesize(text, None).await
            }
        }
    }

    async fn prewarm(&self) {
        if !Self::available() {
            tracing::info!("[tts-mlx] tts venv not ready; the system voice remains");
            return;
        }
        if let Err(e) = self.sidecar.warm().await {
            tracing::warn!("[tts-mlx] pre-warm failed ({e:#}); the system voice remains");
        }
    }

    fn prewarm_on_boot(&self) -> bool {
        true
    }
}

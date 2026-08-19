//! Qwen3-TTS on the Mac's GPU, spoken through a resident Python sidecar.
//!
//! MLX is only practically callable from Python, so the model runs in a
//! child process on the managed runtime's `tts` venv (see
//! [`crate::runtime`]) and the engine talks JSON lines over its stdio:
//! one `{text, voice}` request in, one complete base64 WAV out. One
//! request is in flight at a time — the caller holds the child lock, and
//! generation saturates the GPU anyway.
//!
//! Every failure — venv missing (Intel Mac, prewarm not done), sidecar
//! dead, malformed reply, timeout — falls back to [`KokoroProvider`], so
//! the pet always has a voice. The sidecar script is embedded in the
//! binary and rewritten to disk at spawn, so upgrades ride the engine.

use std::process::Stdio;
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

use super::tts::{KokoroProvider, TtsProvider};

const SIDECAR_SRC: &str = include_str!("../../runtime_py/mlx_tts.py");

/// The model the sidecar serves. Apache-2.0 (upstream Qwen and the MLX
/// conversion both) — license-checked 2026-08-19.
const MODEL: &str = "mlx-community/Qwen3-TTS-12Hz-0.6B-CustomVoice-4bit";

/// Multilingual (zh + en in one timbre), so unlike Kokoro no script→voice
/// mapping is needed. Names must match the model's speaker roster.
const DEFAULT_VOICE: &str = "vivian";
const SPEAKERS: [&str; 9] = [
    "serena", "vivian", "uncle_fu", "ryan", "aiden", "ono_anna", "sohee", "eric", "dylan",
];

const READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Per-clip budget, scaled to the text: healthy synthesis runs ~0.3×
/// realtime, but the GPU can degrade machine-wide (thermal / competing
/// Metal work — seen live 2026-08-19 at ~3× realtime), and a fixed 60s
/// bound meant a minute of silence before Kokoro spoke. Scaling by length
/// keeps the wait proportional: give up early on a short line, allow a
/// long one its honest time.
fn synth_timeout(text: &str) -> std::time::Duration {
    let chars = text.chars().count() as u64;
    std::time::Duration::from_secs((12 + chars / 3).clamp(15, 60))
}

struct Sidecar {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
}

pub struct MlxTtsProvider {
    sidecar: Arc<Mutex<Option<Sidecar>>>,
    fallback: KokoroProvider,
}

impl MlxTtsProvider {
    pub fn new() -> Self {
        Self {
            sidecar: Arc::new(Mutex::new(None)),
            fallback: KokoroProvider::new(),
        }
    }

    fn available() -> bool {
        crate::runtime::env_bin("tts", "python3").exists()
    }

    async fn spawn() -> anyhow::Result<Sidecar> {
        let script = crate::runtime::runtime_dir().join("mlx_tts.py");
        tokio::fs::write(&script, SIDECAR_SRC).await?;

        let mut child = tokio::process::Command::new(crate::runtime::env_bin("tts", "python3"))
            .arg(&script)
            .arg(MODEL)
            .env(
                "HF_HOME",
                crate::paths::linggen_home().join("models/hf-hub"),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
        // The sidecar's stderr (HF progress, MLX chatter) goes to debug logs
        // instead of vanishing — release-print invisibility bites otherwise.
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!("[tts-mlx] {line}");
                }
            });
        }

        let mut sidecar = Sidecar {
            child,
            stdin,
            stdout,
        };
        let ready = tokio::time::timeout(READY_TIMEOUT, read_json(&mut sidecar.stdout)).await??;
        if ready.get("ready").and_then(|v| v.as_bool()) != Some(true) {
            anyhow::bail!("sidecar first line was not ready: {ready}");
        }
        tracing::info!("[tts-mlx] sidecar ready ({MODEL})");
        Ok(sidecar)
    }

    /// One round-trip against the (spawned-if-needed) sidecar. Any error
    /// leaves the slot empty so the next call respawns from scratch.
    async fn request(&self, text: &str, voice: &str) -> anyhow::Result<Vec<u8>> {
        // Bounded waits on the way in: if the sidecar is mid-spawn (boot
        // prewarm holds the lock for up to READY_TIMEOUT) or another clip
        // is hogging it, give up fast — warm Kokoro beats a silent queue.
        let mut guard =
            tokio::time::timeout(std::time::Duration::from_secs(5), self.sidecar.lock())
                .await
                .map_err(|_| anyhow::anyhow!("sidecar busy (spawning or mid-clip)"))?;
        if guard.is_none() {
            *guard = Some(
                tokio::time::timeout(std::time::Duration::from_secs(15), Self::spawn())
                    .await
                    .map_err(|_| anyhow::anyhow!("sidecar spawn too slow"))??,
            );
        }
        let sidecar = guard.as_mut().expect("just spawned");

        let result = tokio::time::timeout(synth_timeout(text), async {
            let req = serde_json::json!({ "text": text, "voice": voice });
            sidecar
                .stdin
                .write_all(format!("{req}\n").as_bytes())
                .await?;
            sidecar.stdin.flush().await?;
            read_json(&mut sidecar.stdout).await
        })
        .await;

        let reply = match result {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return self.kill_and_err(&mut guard, e).await,
            Err(_) => {
                return self
                    .kill_and_err(&mut guard, anyhow::anyhow!("synthesis timed out"))
                    .await
            }
        };
        if reply.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            // The sidecar survived and reported a per-request error — keep it.
            anyhow::bail!(
                "sidecar error: {}",
                reply.get("error").and_then(|v| v.as_str()).unwrap_or("?")
            );
        }
        let b64 = reply
            .get("wav_b64")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("reply missing wav_b64"))?;
        Ok(base64::engine::general_purpose::STANDARD.decode(b64)?)
    }
}

impl MlxTtsProvider {
    /// Kill the sidecar and start a replacement in the background, so the
    /// clip after a failure doesn't also pay the ~3.5s respawn.
    async fn kill_and_err(
        &self,
        guard: &mut Option<Sidecar>,
        e: anyhow::Error,
    ) -> anyhow::Result<Vec<u8>> {
        if let Some(mut s) = guard.take() {
            let _ = s.child.kill().await;
        }
        let slot = self.sidecar.clone();
        tokio::spawn(async move {
            // Locks after the caller's guard drops; a request that raced in
            // first will have respawned already, and this becomes a no-op.
            let mut guard = slot.lock().await;
            if guard.is_none() {
                match Self::spawn().await {
                    Ok(s) => *guard = Some(s),
                    Err(e) => tracing::warn!("[tts-mlx] background respawn failed: {e:#}"),
                }
            }
        });
        Err(e)
    }
}

async fn read_json(
    stdout: &mut BufReader<tokio::process::ChildStdout>,
) -> anyhow::Result<serde_json::Value> {
    let mut line = String::new();
    if stdout.read_line(&mut line).await? == 0 {
        anyhow::bail!("sidecar closed stdout");
    }
    Ok(serde_json::from_str(line.trim())?)
}

#[async_trait]
impl TtsProvider for MlxTtsProvider {
    async fn synthesize(&self, text: &str, voice: Option<&str>) -> anyhow::Result<Vec<u8>> {
        if !Self::available() {
            // Intel Mac, or the runtime prewarm hasn't landed yet.
            return self.fallback.synthesize(text, voice).await;
        }
        // Kokoro voice ids (af_*/zf_*) mean the caller wasn't talking to us.
        let voice = match voice {
            Some(v) if SPEAKERS.contains(&v) => v,
            _ => DEFAULT_VOICE,
        };
        match self.request(text, voice).await {
            Ok(wav) => Ok(wav),
            Err(e) => {
                tracing::warn!("[tts-mlx] {e:#}; falling back to Kokoro for this clip");
                self.fallback.synthesize(text, None).await
            }
        }
    }

    async fn prewarm(&self) {
        // The fallback first: Kokoro is what speaks while the sidecar (or
        // the whole runtime download) is still coming up.
        self.fallback.prewarm().await;
        if !Self::available() {
            tracing::info!("[tts-mlx] tts venv not ready; Kokoro remains the voice");
            return;
        }
        let mut guard = self.sidecar.lock().await;
        if guard.is_none() {
            match Self::spawn().await {
                Ok(s) => *guard = Some(s),
                Err(e) => tracing::warn!("[tts-mlx] pre-warm failed ({e:#}); Kokoro remains"),
            }
        }
    }

    fn prewarm_on_boot(&self) -> bool {
        true
    }
}

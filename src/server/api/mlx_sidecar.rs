//! The one resident Python process of the voice lane, shared by her voice
//! ([`super::tts_mlx`]) and her hearing ([`super::stt_mlx`]).
//!
//! MLX is only practically callable from Python, so the models run in one
//! child process on the managed runtime's `tts` venv (see
//! [`crate::runtime`]) and the engine talks JSON lines over its stdio. One
//! request is in flight at a time: [`VoiceSidecar::ask`] holds the slot's
//! lock across the write and the read, and generation saturates the GPU
//! anyway. The script is embedded in the binary and rewritten to disk at
//! spawn, so upgrades ride the engine; the models it serves are named once,
//! in [`crate::runtime`].
//!
//! Any failure — dead child, malformed reply, timeout — kills the child and
//! starts a replacement in the background; what a caller does about it
//! (Kokoro for her voice, a spoken "try again" for her hearing) is theirs.
//! A request is never logged: it may be a person's voice.

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

const SIDECAR_SRC: &str = include_str!("../../runtime_py/mlx_voice.py");

const READY_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a request waits for the slot: mid-spawn (boot prewarm holds the
/// lock for up to [`READY_TIMEOUT`]) or another request is on it. Give up
/// fast — warm Kokoro beats a silent queue.
const SLOT_WAIT: Duration = Duration::from_secs(5);
/// How long a request waits for a sidecar to be started in front of it.
const SPAWN_WAIT: Duration = Duration::from_secs(15);

struct Sidecar {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
}

pub(crate) struct VoiceSidecar {
    slot: Arc<Mutex<Option<Sidecar>>>,
    /// A program to start instead of the runtime's python: set by tests alone
    /// (`with_program`), which stand a fake of the protocol in its place.
    program: Option<(std::path::PathBuf, Vec<String>)>,
}

impl VoiceSidecar {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self { slot: Arc::new(Mutex::new(None)), program: None })
    }

    /// A sidecar made of `program args…` instead of the runtime's python.
    #[cfg(test)]
    pub(super) fn with_program(program: std::path::PathBuf, args: Vec<String>) -> Arc<Self> {
        Arc::new(Self { slot: Arc::new(Mutex::new(None)), program: Some((program, args)) })
    }

    async fn command(&self) -> anyhow::Result<tokio::process::Command> {
        if let Some((program, args)) = &self.program {
            let mut cmd = tokio::process::Command::new(program);
            cmd.args(args);
            return Ok(cmd);
        }
        let dir = crate::runtime::runtime_dir();
        remove_old_script(&dir).await;
        let script = dir.join("mlx_voice.py");
        tokio::fs::write(&script, SIDECAR_SRC).await?;
        let mut cmd = tokio::process::Command::new(crate::runtime::env_bin("tts", "python3"));
        cmd.arg(&script)
            .arg(crate::runtime::TTS_MODEL)
            .arg(crate::runtime::STT_MODEL)
            .env("HF_HOME", crate::runtime::hf_home())
            .env("HF_ENDPOINT", crate::hf_mirror::endpoint().await);
        Ok(cmd)
    }

    async fn spawn(&self) -> anyhow::Result<Sidecar> {
        let mut child = self
            .command()
            .await?
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
                    tracing::debug!("[voice-sidecar] {line}");
                }
            });
        }

        let mut sidecar = Sidecar { child, stdin, stdout };
        let ready = tokio::time::timeout(READY_TIMEOUT, read_json(&mut sidecar.stdout)).await??;
        if ready.get("ready").and_then(|v| v.as_bool()) != Some(true) {
            anyhow::bail!("sidecar first line was not ready: {ready}");
        }
        tracing::info!("[voice-sidecar] ready");
        Ok(sidecar)
    }

    /// Start the sidecar now if none is running (boot prewarm).
    pub(super) async fn warm(&self) -> anyhow::Result<()> {
        let mut guard = self.slot.lock().await;
        if guard.is_none() {
            *guard = Some(self.spawn().await?);
        }
        Ok(())
    }

    /// One round-trip against the (spawned-if-needed) sidecar: `request` in,
    /// the reply's JSON out once it says `ok`. An error from the child's
    /// own loop (`{"ok": false}`) leaves the sidecar running; anything else
    /// empties the slot so the next call starts afresh.
    pub(super) async fn ask(
        self: &Arc<Self>,
        request: &serde_json::Value,
        timeout: Duration,
    ) -> anyhow::Result<serde_json::Value> {
        let mut guard = tokio::time::timeout(SLOT_WAIT, self.slot.lock())
            .await
            .map_err(|_| anyhow::anyhow!("sidecar busy (spawning or mid-request)"))?;
        if guard.is_none() {
            *guard = Some(
                tokio::time::timeout(SPAWN_WAIT, self.spawn())
                    .await
                    .map_err(|_| anyhow::anyhow!("sidecar spawn too slow"))??,
            );
        }
        let sidecar = guard.as_mut().expect("just spawned");

        let result = tokio::time::timeout(timeout, async {
            sidecar.stdin.write_all(format!("{request}\n").as_bytes()).await?;
            sidecar.stdin.flush().await?;
            read_json(&mut sidecar.stdout).await
        })
        .await;

        let reply = match result {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return self.kill_and_err(&mut guard, e).await,
            Err(_) => {
                return self.kill_and_err(&mut guard, anyhow::anyhow!("request timed out")).await
            }
        };
        if reply.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            anyhow::bail!(
                "sidecar error: {}",
                reply.get("error").and_then(|v| v.as_str()).unwrap_or("?")
            );
        }
        Ok(reply)
    }

    /// Kill the sidecar and start a replacement in the background, so the
    /// request after a failure doesn't also pay the respawn.
    async fn kill_and_err(
        self: &Arc<Self>,
        guard: &mut Option<Sidecar>,
        e: anyhow::Error,
    ) -> anyhow::Result<serde_json::Value> {
        if let Some(mut s) = guard.take() {
            let _ = s.child.kill().await;
        }
        let car = self.clone();
        tokio::spawn(async move {
            // Locks after the caller's guard drops; a request that raced in
            // first will have respawned already, and this becomes a no-op.
            let mut guard = car.slot.lock().await;
            if guard.is_none() {
                match car.spawn().await {
                    Ok(s) => *guard = Some(s),
                    Err(e) => tracing::warn!("[voice-sidecar] background respawn failed: {e:#}"),
                }
            }
        });
        Err(e)
    }
}

/// The voice-only script this one replaced, `mlx_tts.py`: the engine wrote it
/// to the runtime folder at every spawn, its first line says so, and nothing
/// reads it now. Removed only when that first line is there — a file of the
/// same name that something else put in the folder is not the engine's to delete.
async fn remove_old_script(dir: &std::path::Path) {
    let old = dir.join("mlx_tts.py");
    let Ok(text) = tokio::fs::read_to_string(&old).await else { return };
    if text.starts_with(OLD_SCRIPT_HEADER) {
        let _ = tokio::fs::remove_file(&old).await;
    }
}

/// The first line the engine's `mlx_tts.py` carried.
const OLD_SCRIPT_HEADER: &str = "# mlx_tts.py — resident TTS sidecar for the Linggen engine.";

async fn read_json(
    stdout: &mut BufReader<tokio::process::ChildStdout>,
) -> anyhow::Result<serde_json::Value> {
    let mut line = String::new();
    if stdout.read_line(&mut line).await? == 0 {
        anyhow::bail!("sidecar closed stdout");
    }
    Ok(serde_json::from_str(line.trim())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A program that speaks the protocol: ready, then one canned reply per
    /// request line. `replies` are shell-quoted JSON lines used in turn; the
    /// last repeats.
    fn fake(dir: &std::path::Path, replies: &[&str]) -> Arc<VoiceSidecar> {
        let mut script = String::from("echo '{\"ready\":true}'\n");
        for (i, reply) in replies.iter().enumerate() {
            let sep = if i + 1 == replies.len() { "while" } else { "" };
            if sep == "while" {
                script.push_str(&format!("while read line; do echo '{reply}'; done\n"));
            } else {
                script.push_str(&format!("read line; echo '{reply}'\n"));
            }
        }
        let path = dir.join("fake-sidecar.sh");
        std::fs::write(&path, script).unwrap();
        VoiceSidecar::with_program("/bin/sh".into(), vec![path.display().to_string()])
    }

    const SECS: Duration = Duration::from_secs(10);

    /// The engine's own old script goes; a file of that name it did not write stays.
    #[tokio::test]
    async fn the_old_voice_script_is_removed_only_when_the_engine_wrote_it() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("mlx_tts.py");
        std::fs::write(&old, format!("{OLD_SCRIPT_HEADER}\n# Written to ~/.linggen/runtime/ …\nimport sys\n")).unwrap();
        remove_old_script(dir.path()).await;
        assert!(!old.exists(), "the engine's old script is removed");

        std::fs::write(&old, "# my own notes\n").unwrap();
        remove_old_script(dir.path()).await;
        assert!(old.exists(), "a file the engine did not write stays");

        remove_old_script(&dir.path().join("nowhere")).await; // nothing there: nothing happens
    }

    #[tokio::test]
    async fn a_request_gets_the_sidecars_reply() {
        let dir = tempfile::tempdir().unwrap();
        let car = fake(dir.path(), &[r#"{"ok":true,"text":"hello","language":"en"}"#]);
        let reply = car.ask(&serde_json::json!({"op": "hear"}), SECS).await.unwrap();
        assert_eq!(reply["text"], "hello");
        let again = car.ask(&serde_json::json!({"op": "hear"}), SECS).await.unwrap();
        assert_eq!(again["language"], "en");
    }

    /// The child's own error is that request's, and the sidecar goes on.
    #[tokio::test]
    async fn an_error_from_the_child_leaves_it_running() {
        let dir = tempfile::tempdir().unwrap();
        let car = fake(
            dir.path(),
            &[r#"{"ok":false,"error":"no model"}"#, r#"{"ok":true,"text":"back"}"#],
        );
        let failed = car.ask(&serde_json::json!({}), SECS).await.unwrap_err();
        assert!(format!("{failed}").contains("sidecar error: no model"), "{failed}");
        let next = car.ask(&serde_json::json!({}), SECS).await.unwrap();
        assert_eq!(next["text"], "back");
    }

    /// No reply in time: the request fails, never hangs.
    #[tokio::test]
    async fn a_sidecar_that_does_not_answer_times_the_request_out() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mute.sh");
        std::fs::write(&path, "echo '{\"ready\":true}'\nsleep 30\n").unwrap();
        let car = VoiceSidecar::with_program("/bin/sh".into(), vec![path.display().to_string()]);
        let failed = car.ask(&serde_json::json!({}), Duration::from_millis(300)).await;
        assert!(failed.unwrap_err().to_string().contains("timed out"));
    }
}

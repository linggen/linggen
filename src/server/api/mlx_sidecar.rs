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
use std::sync::atomic::{AtomicBool, Ordering};
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
/// How long a hear (or the hearing model's load) waits for the slot: a clip of
/// her voice on it can take up to a minute, and a person who has just spoken
/// is owed more patience than the next line of hers. Worst case, this plus the
/// hear's own wait must fit the page's budget for one hearing
/// (`HEAR_BUDGET_MS`, held by `stt_mlx`'s lock).
pub(super) const HEAR_SLOT_WAIT: Duration = Duration::from_secs(20);
/// How long the hearing model's load may take: reading 1.6 GB cold and
/// compiling the first kernels. A load that is merely slow is not a dead
/// sidecar, so this is long; it runs in its own task, never in a request.
const LOAD_WAIT: Duration = Duration::from_secs(600);

/// How long a request waits: for the slot, then for the reply.
#[derive(Clone, Copy)]
struct Wait {
    slot: Duration,
    reply: Duration,
}

/// What a successful reply tells the sidecar's owner about the child.
#[derive(Clone, Copy, PartialEq)]
enum Mark {
    Nothing,
    HearingLoaded,
}

/// The two lines the script writes on stderr around a hear (`mlx_voice.py`,
/// `HEAR_BEGIN`/`HEAR_END`): what the library writes between them may hold
/// the person's words, so it is counted and never logged.
const HEAR_BEGIN: &str = "\u{1e}hear-begin";
const HEAR_END: &str = "\u{1e}hear-end";

/// What of the child's stderr is logged: its lines as they come, except those
/// between a hear's two marker lines, which are only counted. Order on one
/// pipe is the order written, so no flag shared with another task can race it.
#[derive(Default)]
struct StderrLog {
    in_hear: bool,
    counted: usize,
}

impl StderrLog {
    /// The text to log for `line`, if any.
    fn take(&mut self, line: &str) -> Option<String> {
        match (line, self.in_hear) {
            (HEAR_BEGIN, _) => {
                self.in_hear = true;
                self.counted = 0;
                None
            }
            (HEAR_END, _) => {
                self.in_hear = false;
                Some(format!("{} lines on stderr during a hear (not logged)", self.counted))
            }
            (_, true) => {
                self.counted += 1;
                None
            }
            (text, false) => Some(text.to_string()),
        }
    }
}

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
    /// The child in the slot has the hearing model loaded. Set and cleared
    /// only with the slot's lock held (a reply that loaded it; a kill; a
    /// spawn), so it is never the answer for a child that is gone.
    hearing_loaded: AtomicBool,
}

impl VoiceSidecar {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self { slot: Arc::new(Mutex::new(None)), program: None, hearing_loaded: AtomicBool::new(false) })
    }

    /// Is the hearing model loaded in the child now running? Asking starts nothing.
    pub(super) fn hearing_loaded(&self) -> bool {
        self.hearing_loaded.load(Ordering::SeqCst)
    }

    /// A sidecar made of `program args…` instead of the runtime's python.
    #[cfg(test)]
    pub(super) fn with_program(program: std::path::PathBuf, args: Vec<String>) -> Arc<Self> {
        Arc::new(Self {
            slot: Arc::new(Mutex::new(None)),
            program: Some((program, args)),
            hearing_loaded: AtomicBool::new(false),
        })
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
        self.hearing_loaded.store(false, Ordering::SeqCst); // a new child has loaded nothing
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
        // instead of vanishing — release-print invisibility bites otherwise —
        // except what lies inside a hear (`StderrLog`).
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                let mut log = StderrLog::default();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(text) = log.take(&line) {
                        tracing::debug!("[voice-sidecar] {text}");
                    }
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

    /// One round-trip for her voice: waits `SLOT_WAIT` for the slot, `timeout`
    /// for the reply; `request` in, the reply's JSON out once it says `ok`. An
    /// error from the child's own loop (`{"ok": false}`) leaves the sidecar
    /// running; anything else empties the slot so the next call starts afresh.
    pub(super) async fn ask(
        self: &Arc<Self>,
        request: &serde_json::Value,
        timeout: Duration,
    ) -> anyhow::Result<serde_json::Value> {
        self.exchange(request, Wait { slot: SLOT_WAIT, reply: timeout }, Mark::Nothing).await
    }

    /// One round-trip for a hear: the same, but it waits [`HEAR_SLOT_WAIT`] for
    /// the slot, so a clip of her voice in front of it does not fail it.
    pub(super) async fn ask_to_hear(
        self: &Arc<Self>,
        request: &serde_json::Value,
        timeout: Duration,
    ) -> anyhow::Result<serde_json::Value> {
        self.exchange(request, Wait { slot: HEAR_SLOT_WAIT, reply: timeout }, Mark::Nothing).await
    }

    /// Have the child load the hearing model, and wait as long as that takes
    /// ([`LOAD_WAIT`]). The sidecar is started first with no short limit, and
    /// the reply is read here to its end — a load abandoned half way would
    /// leave its reply to be read as the next request's. The caller runs this
    /// in a task of its own.
    pub(super) async fn load_hearing(self: &Arc<Self>) -> anyhow::Result<()> {
        self.warm().await?;
        let wait = Wait { slot: HEAR_SLOT_WAIT, reply: LOAD_WAIT };
        self.exchange(&serde_json::json!({ "op": "load" }), wait, Mark::HearingLoaded).await?;
        Ok(())
    }

    async fn exchange(
        self: &Arc<Self>,
        request: &serde_json::Value,
        wait: Wait,
        mark: Mark,
    ) -> anyhow::Result<serde_json::Value> {
        let mut guard = tokio::time::timeout(wait.slot, self.slot.lock())
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

        let result = tokio::time::timeout(wait.reply, async {
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
        if mark == Mark::HearingLoaded {
            self.hearing_loaded.store(true, Ordering::SeqCst); // slot's lock still held
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
        self.hearing_loaded.store(false, Ordering::SeqCst);
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

    /// A load is its own request, and "loaded" belongs to the child that
    /// answered it: a child killed after a timeout has loaded nothing.
    #[tokio::test]
    async fn loaded_is_the_running_childs_and_goes_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("loads.sh");
        std::fs::write(
            &path,
            "echo '{\"ready\":true}'\nread line; echo '{\"ok\":true}'\nread line; sleep 30\n",
        )
        .unwrap();
        let car = VoiceSidecar::with_program("/bin/sh".into(), vec![path.display().to_string()]);
        assert!(!car.hearing_loaded());
        car.load_hearing().await.unwrap();
        assert!(car.hearing_loaded());
        // Her voice times out on the same child: it is killed, and with it what it had loaded.
        let failed = car.ask(&serde_json::json!({ "text": "hi" }), Duration::from_millis(300)).await;
        assert!(failed.unwrap_err().to_string().contains("timed out"));
        assert!(!car.hearing_loaded(), "a replaced child has loaded nothing");
    }

    /// A child that fails its load is not loaded, and keeps running.
    #[tokio::test]
    async fn a_failed_load_is_not_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let car = fake(dir.path(), &[r#"{"ok":false,"error":"no model"}"#]);
        assert!(car.load_hearing().await.is_err());
        assert!(!car.hearing_loaded());
    }

    /// What a hear writes on stderr is counted, never logged; what else the
    /// child writes still is.
    #[test]
    fn a_hears_stderr_is_counted_and_never_logged() {
        let mut log = StderrLog::default();
        assert_eq!(log.take("loading voice").as_deref(), Some("loading voice"));
        assert_eq!(log.take(HEAR_BEGIN), None);
        assert_eq!(log.take("今天有什么安排"), None, "words that may be the person's");
        assert_eq!(log.take("100%|███| 30/30"), None);
        let closing = log.take(HEAR_END).unwrap();
        assert_eq!(closing, "2 lines on stderr during a hear (not logged)");
        assert_eq!(log.take("after").as_deref(), Some("after"));
        // The next hear counts from zero.
        log.take(HEAR_BEGIN);
        assert_eq!(log.take(HEAR_END).unwrap(), "0 lines on stderr during a hear (not logged)");
    }

    /// The two markers are the script's: one pair, in both places.
    #[test]
    fn the_stderr_markers_are_the_scripts_own() {
        assert!(SIDECAR_SRC.contains(r#"HEAR_BEGIN = "\x1ehear-begin""#));
        assert!(SIDECAR_SRC.contains(r#"HEAR_END = "\x1ehear-end""#));
        assert_eq!((HEAR_BEGIN, HEAR_END), ("\u{1e}hear-begin", "\u{1e}hear-end"));
    }

    /// A hear waits longer for the slot than her voice does.
    #[test]
    fn a_hear_waits_longer_for_the_slot_than_her_voice() {
        assert!(HEAR_SLOT_WAIT > SLOT_WAIT);
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

//! A foreground Bash: the turn waits for the command and reads its output.
//! It waits on the child's end, its timeout, a kill from the chat, and the
//! turn's Steer. Steer and the foreground limit both hand a running command
//! to the session's background tasks (`background::watch_shell`). Its output
//! goes to the task's file and the call returns.
use super::search_exec::{find_git_root, kill_process_group, RunCommandArgs};
use super::{ToolProgressSender, ToolResult, Tools};
use crate::util::{LockExt, CWD_SENTINEL};
use anyhow::Result;
use std::io::{BufRead, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// After Steer, how long a command may still end on its own before it moves:
/// one about to finish is read as usual.
#[cfg(not(test))]
const STEER_GRACE: Duration = Duration::from_secs(1);
#[cfg(test)]
const STEER_GRACE: Duration = Duration::from_millis(100);

/// Once the shell has ended, how long its pipes may stay open before the
/// call returns without them: a process the command put in the background
/// (`server &`) holds them for as long as it runs. Only such a call waits
/// it out; any other's readers end at once.
const EXIT_DRAIN: Duration = Duration::from_millis(1500);

/// Of each stream, what is kept while it is read: its first and its last
/// this many bytes — all the result can show (`cap_parts`), never more.
const KEEP: usize = super::DEFAULT_MAX_TOOL_OUTPUT_BYTES;

/// The longest piece of one line read at a time: a line longer than this
/// comes in pieces, so one endless line is never held whole either.
const LINE_MAX: u64 = KEEP as u64;

const LEFT_RUNNING: &str = "linggen: the shell ended, but a process it started in the \
background still holds its output and was left running; this is the output so far. \
Redirect its output (`cmd > file 2>&1 &`) or use run_in_background.";

/// How the wait for the command ended.
enum Waited {
    Exited,
    TimedOut,
    Interrupted,
    Steered,
}

/// What a foreground Bash came to.
pub(super) enum Ran {
    Done(ToolResult),
    Moved(Moved),
}

/// A command that left the foreground, ready to be watched as a background task.
pub(super) struct Moved {
    child: Child,
    readers: Vec<JoinHandle<()>>,
    id: String,
    output: PathBuf,
    cmd: String,
    started: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stream {
    Stdout,
    Stderr,
}

/// One stream as it is read: its first [`KEEP`] bytes, the last [`KEEP`]
/// after those (up to twice that between trims), and how many came in all.
#[derive(Default)]
struct Kept {
    head: String,
    tail: String,
    total: usize,
}

impl Kept {
    fn push(&mut self, mut text: &str) {
        self.total += text.len();
        if self.tail.is_empty() {
            let cut = super::floor_char_boundary(text, KEEP.saturating_sub(self.head.len()));
            self.head.push_str(&text[..cut]);
            text = &text[cut..];
        }
        self.tail.push_str(text);
        if self.tail.len() > 2 * KEEP {
            let start = super::ceil_char_boundary(&self.tail, self.tail.len() - KEEP);
            self.tail.drain(..start);
        }
    }

    fn whole(&self) -> bool {
        self.total == self.head.len() + self.tail.len()
    }

    /// What the result shows of it, `strip` applied to its end first.
    fn shown(self, strip: impl FnOnce(String) -> String) -> String {
        let max = super::DEFAULT_MAX_TOOL_OUTPUT_BYTES;
        if self.whole() {
            return super::cap_stream(&strip(self.head + &self.tail), max);
        }
        let kept_tail = self.tail.len();
        let tail = strip(self.tail);
        let total = self.total - kept_tail + tail.len();
        super::cap_parts(&self.head, &tail, total, max)
    }
}

/// What the command printed. Until it moves, lines are kept for the result
/// and streamed to the chat; once moved, they go to the task's file.
#[derive(Default)]
struct Capture {
    stdout: Kept,
    stderr: Kept,
    log: Option<std::fs::File>,
    /// The cwd sentinel came: what follows is the shell's `pwd`, not output.
    sentinel_seen: bool,
    /// The call has returned: a reader still open drops what comes.
    closed: bool,
}

impl Capture {
    /// One line, or (`ended` false) a piece of a line too long to read whole.
    fn take_line(
        &mut self,
        stream: Stream,
        line: String,
        ended: bool,
        progress: Option<&ToolProgressSender>,
    ) {
        if self.closed {
            return;
        }
        if self.log.is_some() {
            return self.log_line(stream, &line, ended);
        }
        if let Some(tx) = progress {
            let name = if stream == Stream::Stdout {
                "stdout"
            } else {
                "stderr"
            };
            let _ = tx.send(("Bash".to_string(), name.to_string(), line.clone()));
        }
        let acc = if stream == Stream::Stdout {
            &mut self.stdout
        } else {
            &mut self.stderr
        };
        acc.push(&line);
        if ended {
            acc.push("\n");
        }
    }

    /// A moved command's line, without the cwd sentinel and its `pwd`.
    fn log_line(&mut self, stream: Stream, line: &str, ended: bool) {
        let mut text = line;
        if stream == Stream::Stdout {
            if self.sentinel_seen {
                return;
            }
            if let Some(at) = line.find(CWD_SENTINEL) {
                self.sentinel_seen = true;
                text = &line[..at];
                if text.is_empty() {
                    return;
                }
            }
        }
        if let Some(log) = self.log.as_mut() {
            let _ = log.write_all(text.as_bytes());
            if ended {
                let _ = log.write_all(b"\n");
            }
        }
    }

    /// From now on lines go to `log`, which starts with what came so far.
    fn move_to(&mut self, mut log: std::fs::File) -> std::io::Result<()> {
        for kept in [&self.stdout, &self.stderr] {
            log.write_all(kept.head.as_bytes())?;
            if !kept.whole() {
                let dropped = kept.total - kept.head.len() - kept.tail.len();
                writeln!(log, "\n[linggen: {dropped} bytes omitted]")?;
            }
            log.write_all(kept.tail.as_bytes())?;
        }
        self.stdout = Kept::default();
        self.stderr = Kept::default();
        self.log = Some(log);
        Ok(())
    }
}

fn read_lines(
    pipe: Option<impl Read + Send + 'static>,
    stream: Stream,
    capture: Arc<Mutex<Capture>>,
    progress: Option<ToolProgressSender>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let Some(pipe) = pipe else { return };
        let mut reader = std::io::BufReader::new(pipe);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match (&mut reader).take(LINE_MAX).read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            // A piece cut at LINE_MAX goes on in the next read; anything
            // else (a newline, the end of the stream) ends the line.
            let ended = buf.last() == Some(&b'\n') || (buf.len() as u64) < LINE_MAX;
            if buf.last() == Some(&b'\n') {
                buf.pop();
                if buf.last() == Some(&b'\r') {
                    buf.pop();
                }
            }
            let line = String::from_utf8_lossy(&buf).into_owned();
            capture.lock_ok().take_line(stream, line, ended, progress.as_ref());
        }
    })
}

/// Wait up to [`EXIT_DRAIN`] for every reader to reach its pipe's end;
/// false when one is still held open.
fn pipes_drained(readers: &[JoinHandle<()>]) -> bool {
    let until = Instant::now() + EXIT_DRAIN;
    while !readers.iter().all(JoinHandle::is_finished) {
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    true
}

fn foreground_limit(timeout_ms: Option<u64>) -> Duration {
    Duration::from_millis(timeout_ms.unwrap_or(120_000).min(600_000))
}

impl Tools {
    pub(super) fn run_foreground_shell(&self, args: RunCommandArgs) -> Result<Ran> {
        let timeout = foreground_limit(args.timeout_ms);
        let cwd = self.shell_cwd();
        // Wrap the user command to capture the final working directory.
        // Preserves the original exit code while appending a sentinel + pwd.
        let wrapped_cmd = crate::util::wrap_with_cwd_sentinel(&args.cmd);
        let mut child = self.start_foreground(&args.cmd, &wrapped_cmd, &cwd)?;

        let capture = Arc::new(Mutex::new(Capture::default()));
        let readers = vec![
            read_lines(
                child.stdout.take(),
                Stream::Stdout,
                capture.clone(),
                self.progress_tx.clone(),
            ),
            read_lines(
                child.stderr.take(),
                Stream::Stderr,
                capture.clone(),
                self.progress_tx.clone(),
            ),
        ];

        let deadline = Instant::now() + timeout;
        let mut allow_steer = self.steer.is_some();
        let note = loop {
            let waited = self.wait_child(
                &mut child,
                deadline,
                args.cancel_flag.as_deref(),
                allow_steer,
            )?;
            match waited {
                Waited::Exited => break None,
                Waited::Interrupted => {
                    break Some("linggen: command interrupted by user".to_string());
                }
                Waited::TimedOut | Waited::Steered => {}
            }
            match self.move_capture(&capture) {
                Ok((id, output)) => {
                    return Ok(Ran::Moved(Moved {
                        child,
                        readers,
                        id,
                        output,
                        cmd: args.cmd,
                        started: wrapped_cmd,
                    }));
                }
                Err(error) if matches!(waited, Waited::TimedOut) => {
                    break Some(format!(
                        "linggen: command timed out after {}ms and was killed: {error}",
                        timeout.as_millis()
                    ));
                }
                Err(error) => {
                    tracing::warn!("Steer could not move the command to the background: {error}");
                    allow_steer = false;
                }
            }
        };

        if note.is_some() {
            kill_process_group(&child);
            #[cfg(not(unix))]
            let _ = child.kill();
        }
        let exit_status = child.wait()?;
        // The call returns when the shell has ended, not when every process
        // holding its pipes has: one left in the background keeps running,
        // and its readers drop what it prints from now on.
        let held = !pipes_drained(&readers);
        let note = note.or_else(|| held.then(|| LEFT_RUNNING.to_string()));
        let (stdout, stderr) = {
            let mut capture = capture.lock_ok();
            capture.closed = true;
            (
                std::mem::take(&mut capture.stdout),
                std::mem::take(&mut capture.stderr),
            )
        };
        let stdout = stdout.shown(|s| self.strip_cwd_sentinel(s));
        let mut stderr = stderr.shown(|s| s);
        if let Some(note) = note {
            if !stderr.is_empty() && !stderr.ends_with('\n') {
                stderr.push('\n');
            }
            stderr.push_str(&note);
            stderr.push('\n');
        }
        Ok(Ran::Done(ToolResult::CommandOutput {
            exit_code: exit_status.code(),
            stdout,
            stderr,
        }))
    }

    fn shell_cwd(&self) -> PathBuf {
        let cwd = self.cwd();
        if cwd.is_dir() {
            return cwd;
        }
        tracing::warn!(
            "Bash cwd {:?} does not exist, falling back to root {:?}",
            cwd,
            self.root
        );
        self.root.clone()
    }

    fn start_foreground(
        &self,
        cmd: &str,
        wrapped_cmd: &str,
        cwd: &std::path::Path,
    ) -> Result<Child> {
        if cfg!(target_os = "windows") {
            return Ok(Command::new("cmd")
                .args(["/C", cmd])
                .current_dir(cwd)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?);
        }
        #[cfg(unix)]
        use std::os::unix::process::CommandExt;
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(wrapped_cmd)
            .current_dir(cwd)
            .env("PATH", crate::util::shell_path())
            .env(crate::language::ENV_KEY, crate::language::effective())
            // Which session this shell runs for (`ling restart` reads it).
            .env(
                crate::paths::SESSION_ENV,
                self.session_id.as_deref().unwrap_or_default(),
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        command.process_group(0);
        Ok(command.spawn()?)
    }

    /// Wait for the child's end, its foreground deadline, a cancel, or
    /// (when `allow_steer`) a Steer it did not end within [`STEER_GRACE`] of.
    fn wait_child(
        &self,
        child: &mut Child,
        deadline: Instant,
        cancel: Option<&AtomicBool>,
        allow_steer: bool,
    ) -> Result<Waited> {
        let mut steered_at: Option<Instant> = None;
        loop {
            if child.try_wait()?.is_some() {
                return Ok(Waited::Exited);
            }
            if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                return Ok(Waited::Interrupted);
            }
            if Instant::now() >= deadline {
                return Ok(Waited::TimedOut);
            }
            if allow_steer && steered_at.is_none() && self.steer_pending() {
                steered_at = Some(Instant::now());
            }
            if steered_at.is_some_and(|at| at.elapsed() >= STEER_GRACE) {
                return Ok(Waited::Steered);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// The command's output from now on goes to a new task file.
    fn move_capture(&self, capture: &Mutex<Capture>) -> Result<(String, PathBuf)> {
        let (Some(manager), Some(session)) = (&self.manager, &self.session_id) else {
            anyhow::bail!("no session to keep a background task in");
        };
        let (id, output) = crate::engine::background::new_shell_output(manager, session)?;
        capture.lock_ok().move_to(std::fs::File::create(&output)?)?;
        Ok((id, output))
    }

    /// Hand a moved command to the session's background tasks and say so.
    pub(super) async fn watch_moved(&self, moved: Moved) -> Result<ToolResult> {
        let (Some(manager), Some(session)) = (&self.manager, &self.session_id) else {
            anyhow::bail!("no session to keep a background task in");
        };
        let Moved {
            mut child,
            readers,
            id,
            output,
            cmd,
            started,
        } = moved;
        let pid = Some(child.id());
        let exit = async move {
            let status = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break Some(status),
                    Ok(None) => tokio::time::sleep(Duration::from_millis(200)).await,
                    Err(_) => break None,
                }
            };
            // Every line is in the file before the end is told.
            let _ = tokio::task::spawn_blocking(move || {
                readers.into_iter().for_each(|r| drop(r.join()))
            })
            .await;
            status
        };
        let agent = self.agent_id.clone().unwrap_or_else(|| "ling".into());
        let shell = crate::engine::background::Shell {
            session,
            agent: &agent,
            owner: self.task_owner(),
            child_owner: self.delegation_depth() > 0,
            id,
            cmd: &cmd,
            started,
            output: output.clone(),
            pid,
        };
        let task = crate::engine::background::watch_shell(manager, shell, exit).await;
        let so_far = crate::engine::background::tail(&output, 20);
        Ok(ToolResult::Success(format!(
            "Moved to the background as task {}. Output: {}. You will be told in a new turn \
             when it ends; TaskOutput reads it, TaskStop stops it.\nOutput so far:\n{}",
            task.id,
            output.display(),
            if so_far.is_empty() { "(none)" } else { &so_far }
        )))
    }

    /// Strip the cwd sentinel from stdout and update the session's cwd. Use
    /// substring match (not whole-line) so commands whose final line of
    /// output lacks a trailing newline — e.g. `cat file_without_final_nl`
    /// — still strip cleanly; otherwise the sentinel glues onto the user
    /// content (`}__LINGGEN_CWD__`) and leaks into the tool result.
    fn strip_cwd_sentinel(&self, mut s: String) -> String {
        let sentinel_with_nl = format!("{}\n", CWD_SENTINEL);
        if let Some(sent_pos) = s.rfind(&sentinel_with_nl) {
            let after_sent = sent_pos + sentinel_with_nl.len();
            let pwd_end = s[after_sent..]
                .find('\n')
                .map(|i| after_sent + i)
                .unwrap_or(s.len());
            self.note_cwd(PathBuf::from(&s[after_sent..pwd_end]));
            // Strip [sentinel, end-of-pwd-line] inclusive of trailing \n.
            let strip_end = (pwd_end + 1).min(s.len());
            s.replace_range(sent_pos..strip_end, "");
        }
        // Restore trailing newline if original had one.
        if !s.is_empty() {
            s.push('\n');
        }
        s
    }

    /// The shell ended in `new_cwd`: keep it for the session, and tell the
    /// chat when it changed.
    fn note_cwd(&self, new_cwd: PathBuf) {
        if !new_cwd.is_absolute() || !new_cwd.exists() {
            return;
        }
        let Some(sid) = &self.session_id else { return };
        let old_cwd = self
            .cwd_by_session
            .lock_ok()
            .insert(sid.clone(), new_cwd.clone());
        // A turn in another app's place moves only its own folder: the
        // session's (its meta, the chat's folder line) stays where it is.
        if old_cwd.as_ref() == Some(&new_cwd) || self.in_foreign_place() {
            return;
        }
        let Some(tx) = &self.progress_tx else { return };
        let git_root = find_git_root(&new_cwd);
        let project = git_root
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let project_name = git_root
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let _ = tx.send((
            "__cwd_changed__".to_string(),
            format!("{}|{}", project, project_name),
            new_cwd.to_string_lossy().to_string(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::agent::AgentManager;
    use crate::engine::steer::SteerSignal;

    fn manager_in(dir: &std::path::Path) -> Arc<AgentManager> {
        let (mut manager, _events) = AgentManager::new(
            crate::config::Config::default(),
            None,
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            crate::engine::InterfaceMode::Web,
        );
        Arc::get_mut(&mut manager)
            .expect("a fresh manager")
            .global_sessions =
            crate::state_fs::SessionStore::with_sessions_dir(dir.join("sessions"));
        manager
    }

    fn bash(cmd: &str) -> RunCommandArgs {
        serde_json::from_value(serde_json::json!({"cmd": cmd, "timeout_ms": 20000})).unwrap()
    }

    fn tools_with_steer(dir: &std::path::Path) -> (Tools, Arc<AgentManager>, Arc<SteerSignal>) {
        let manager = manager_in(dir);
        let mut tools = Tools::new(dir.to_path_buf()).unwrap();
        tools.set_context(manager.clone(), "ling".into());
        tools.set_session_id(Some("s".into()));
        let steer = Arc::new(SteerSignal::default());
        tools.set_steer(Some(steer.clone()));
        (tools, manager, steer)
    }

    #[test]
    fn foreground_limit_defaults_to_two_minutes() {
        assert_eq!(foreground_limit(None), Duration::from_secs(120));
    }

    #[test]
    fn foreground_limit_honours_requested_time_up_to_ten_minutes() {
        assert_eq!(foreground_limit(Some(180_000)), Duration::from_secs(180));
        assert_eq!(foreground_limit(Some(600_000)), Duration::from_secs(600));
        assert_eq!(foreground_limit(Some(u64::MAX)), Duration::from_secs(600));
    }

    #[tokio::test]
    async fn deadline_moves_a_running_shell_even_without_steer() {
        let dir = tempfile::tempdir().unwrap();
        let (mut tools, manager, _) = tools_with_steer(dir.path());
        tools.set_steer(None);
        let mut args = bash("echo before; sleep 2; echo after");
        args.timeout_ms = Some(300);
        let result = tools.run_command(args).await.unwrap();
        let ToolResult::Success(text) = result else {
            panic!("expected the deadline to hand off the command: {result:?}");
        };
        assert!(
            text.starts_with("Moved to the background as task sh-"),
            "{text}"
        );
        assert!(text.contains("Output so far:\nbefore"), "{text}");
        let tasks = manager.background.list_owned("s", Some("ling"));
        assert_eq!(tasks.len(), 1);
        assert!(
            tasks[0].is_running(),
            "the deadline did not kill the command"
        );
        let id = &tasks[0].id;
        tokio::time::timeout(Duration::from_secs(5), async {
            while manager.background.get("s", id).unwrap().is_running() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        let task = manager.background.get("s", id).unwrap();
        assert_eq!(task.status, "completed");
        let log = std::fs::read_to_string(task.output_file.unwrap()).unwrap();
        assert_eq!(log, "before\nafter\n");
    }

    #[tokio::test]
    async fn deadline_without_a_session_kills_the_shell_and_explains_why() {
        let dir = tempfile::tempdir().unwrap();
        let tools = Tools::new(dir.path().to_path_buf()).unwrap();
        let mut args = bash("sleep 1; echo survived");
        args.timeout_ms = Some(50);
        let result = tools.run_command(args).await.unwrap();
        let ToolResult::CommandOutput { stdout, stderr, .. } = result else {
            panic!("expected command output");
        };
        assert!(stdout.is_empty(), "{stdout}");
        assert!(stderr.contains("was killed"), "{stderr}");
        assert!(stderr.contains("no session"), "{stderr}");
    }

    #[tokio::test]
    async fn cancellation_wins_over_the_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let (tools, manager, _) = tools_with_steer(dir.path());
        let mut args = bash("sleep 1; echo survived");
        args.timeout_ms = Some(0);
        args.cancel_flag = Some(Arc::new(AtomicBool::new(true)));
        let result = tools.run_command(args).await.unwrap();
        let ToolResult::CommandOutput { stdout, stderr, .. } = result else {
            panic!("expected command output");
        };
        assert!(stdout.is_empty(), "{stdout}");
        assert!(stderr.contains("interrupted by user"), "{stderr}");
        assert!(manager.background.list_owned("s", None).is_empty());
    }

    /// Steer moves a running foreground command to the background without
    /// killing it: it is listed as the session's task, keeps running, and
    /// what it prints after goes to the task's file.
    #[tokio::test]
    async fn steer_hands_a_running_foreground_shell_to_the_background() {
        let dir = tempfile::tempdir().unwrap();
        let (tools, manager, steer) = tools_with_steer(dir.path());
        let call = tokio::spawn({
            let tools = tools.clone();
            async move {
                tools
                    .run_command(bash("echo before; sleep 1; echo after"))
                    .await
            }
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
        steer.fire();
        let result = tokio::time::timeout(Duration::from_secs(5), call)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let ToolResult::Success(text) = result else {
            panic!("expected the move to be told")
        };
        assert!(
            text.starts_with("Moved to the background as task sh-"),
            "{text}"
        );
        assert!(text.contains("before"), "the output so far is told: {text}");

        let tasks = manager.background.list_owned("s", Some("ling"));
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].is_running(), "the command was not killed");
        let id = tasks[0].id.clone();
        tokio::time::timeout(Duration::from_secs(5), async {
            while manager.background.get("s", &id).unwrap().is_running() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        let task = manager.background.get("s", &id).unwrap();
        assert_eq!(task.status, "completed");
        let log = std::fs::read_to_string(task.output_file.unwrap()).unwrap();
        assert_eq!(log, "before\nafter\n", "no cwd sentinel in the file");
    }

    /// A command that ends without a Steer is read as before: its own
    /// output, the cwd sentinel stripped, nothing listed.
    #[tokio::test]
    async fn an_unsteered_command_returns_its_output() {
        let dir = tempfile::tempdir().unwrap();
        let (tools, manager, _steer) = tools_with_steer(dir.path());
        let result = tools
            .run_command(bash("echo hi; echo oops >&2"))
            .await
            .unwrap();
        let ToolResult::CommandOutput { stdout, stderr, .. } = result else {
            panic!("expected output")
        };
        assert_eq!(stdout.trim(), "hi");
        assert_eq!(stderr.trim(), "oops");
        assert!(manager.background.list_owned("s", Some("ling")).is_empty());
    }

    /// A shell that ends while a process it put in the background still
    /// holds its pipes returns at its end, with what it printed and a note;
    /// the background process is left running.
    #[tokio::test]
    async fn a_shell_returns_when_it_ends_though_its_background_child_holds_the_pipes() {
        let dir = tempfile::tempdir().unwrap();
        let tools = Tools::new(dir.path().to_path_buf()).unwrap();
        let call = tools.run_command(bash("echo before; sleep 30 & echo $! > child.pid"));
        let result = tokio::time::timeout(Duration::from_secs(8), call).await;
        let pid: i32 = std::fs::read_to_string(dir.path().join("child.pid"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        let alive = pid > 0 && unsafe { libc::kill(pid, 0) } == 0;
        if pid > 0 {
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        let result = result.expect("the call waited on the background child").unwrap();
        let ToolResult::CommandOutput { stdout, stderr, exit_code } = result else {
            panic!("expected command output");
        };
        assert_eq!(stdout.trim(), "before");
        assert_eq!(exit_code, Some(0));
        assert!(stderr.contains("left running"), "{stderr}");
        assert!(alive, "the background child was killed");
    }

    /// What is kept of a stream while it is read is its head and its tail,
    /// however much comes: the result is the same head-and-tail cap.
    #[test]
    fn a_stream_is_kept_as_head_and_tail_while_read() {
        let mut kept = Kept::default();
        let line = "x".repeat(1000) + "\n";
        for _ in 0..10_000 {
            kept.push(&line);
        }
        kept.push("the end\n");
        assert!(kept.head.len() + kept.tail.len() <= 3 * KEEP);
        let shown = kept.shown(|s| s);
        assert!(shown.starts_with("xxx"), "{}", &shown[..20]);
        assert!(shown.ends_with("the end\n"));
        assert!(shown.contains(&format!("output was {} bytes", 10_000 * 1001 + 8)), "{shown:.200}");
    }

    /// One endless line is read in pieces, never held whole.
    #[tokio::test]
    async fn one_huge_line_comes_back_capped() {
        let dir = tempfile::tempdir().unwrap();
        let tools = Tools::new(dir.path().to_path_buf()).unwrap();
        let result = tools
            .run_command(bash("head -c 3000000 /dev/zero | tr '\\0' a; echo; echo tail-end"))
            .await
            .unwrap();
        let ToolResult::CommandOutput { stdout, .. } = result else {
            panic!("expected command output");
        };
        assert!(stdout.starts_with("aaa"));
        assert!(stdout.trim_end().ends_with("tail-end"), "{}", &stdout[stdout.len() - 50..]);
        assert!(stdout.contains("bytes omitted — output was 3000"), "{stdout:.300}");
    }

    /// A subagent's own Bash never moves on its parent's Steer.
    #[tokio::test]
    async fn a_subagents_shell_ignores_the_steer() {
        let dir = tempfile::tempdir().unwrap();
        let (mut tools, manager, steer) = tools_with_steer(dir.path());
        tools.set_delegation_depth(1);
        steer.fire();
        let result = tools
            .run_command(bash("sleep 0.3; echo end"))
            .await
            .unwrap();
        assert!(matches!(result, ToolResult::CommandOutput { .. }));
        assert!(manager.background.list_owned("s", None).is_empty());
    }
}

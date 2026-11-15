//! A foreground Bash: the turn waits for the command and reads its output.
//! It waits on the child's end, its timeout, a kill from the chat, and the
//! turn's Steer. On Steer the command is not killed: it moves to the
//! background as a task of the session (`background::watch_shell`), its
//! output from then on going to the task's file, and the call returns.
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

/// A command Steer moved, ready to be watched as a background task.
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

/// What the command printed. Until it moves, lines are kept for the result
/// and streamed to the chat; once moved, they go to the task's file.
#[derive(Default)]
struct Capture {
    stdout: String,
    stderr: String,
    log: Option<std::fs::File>,
    /// The cwd sentinel came: what follows is the shell's `pwd`, not output.
    sentinel_seen: bool,
}

impl Capture {
    fn take_line(&mut self, stream: Stream, line: String, progress: Option<&ToolProgressSender>) {
        if self.log.is_some() {
            return self.log_line(stream, &line);
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
        acc.push_str(&line);
        acc.push('\n');
    }

    /// A moved command's line, without the cwd sentinel and its `pwd`.
    fn log_line(&mut self, stream: Stream, line: &str) {
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
            let _ = writeln!(log, "{text}");
        }
    }

    /// From now on lines go to `log`, which starts with what came so far.
    fn move_to(&mut self, mut log: std::fs::File) -> std::io::Result<()> {
        log.write_all(self.stdout.as_bytes())?;
        log.write_all(self.stderr.as_bytes())?;
        self.stdout.clear();
        self.stderr.clear();
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
        for line in std::io::BufReader::new(pipe).lines() {
            let Ok(line) = line else { break };
            capture.lock_ok().take_line(stream, line, progress.as_ref());
        }
    })
}

impl Tools {
    pub(super) fn run_foreground_shell(&self, args: RunCommandArgs) -> Result<Ran> {
        let timeout = Duration::from_millis(args.timeout_ms.unwrap_or(30000));
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

        let mut may_move = self.steer.is_some();
        let waited = loop {
            let waited =
                self.wait_child(&mut child, timeout, args.cancel_flag.as_deref(), may_move)?;
            let Waited::Steered = waited else {
                break waited;
            };
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
                Err(error) => {
                    tracing::warn!("Steer could not move the command to the background: {error}");
                    may_move = false;
                }
            }
        };

        let exit_status = child.wait()?;
        for reader in readers {
            let _ = reader.join();
        }
        let Capture {
            stdout, mut stderr, ..
        } = std::mem::take(&mut *capture.lock_ok());
        let stdout = self.strip_cwd_sentinel(stdout);
        let note = match waited {
            Waited::TimedOut => Some(format!(
                "linggen: command timed out after {}ms",
                timeout.as_millis()
            )),
            Waited::Interrupted => Some("linggen: command interrupted by user".to_string()),
            Waited::Exited | Waited::Steered => None,
        };
        if let Some(note) = note {
            if !stderr.is_empty() && !stderr.ends_with('\n') {
                stderr.push('\n');
            }
            stderr.push_str(&note);
            stderr.push('\n');
        }
        Ok(Ran::Done(ToolResult::command_output(
            exit_status.code(),
            &stdout,
            &stderr,
            super::DEFAULT_MAX_TOOL_OUTPUT_BYTES,
        )))
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

    /// Wait for the child's end, its timeout, a kill, or (when `may_move`)
    /// a Steer it did not end within [`STEER_GRACE`] of.
    fn wait_child(
        &self,
        child: &mut Child,
        timeout: Duration,
        cancel: Option<&AtomicBool>,
        may_move: bool,
    ) -> Result<Waited> {
        let start = Instant::now();
        let mut steered_at: Option<Instant> = None;
        loop {
            if child.try_wait()?.is_some() {
                return Ok(Waited::Exited);
            }
            if start.elapsed() >= timeout {
                kill_process_group(child);
                return Ok(Waited::TimedOut);
            }
            if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                kill_process_group(child);
                return Ok(Waited::Interrupted);
            }
            if may_move && steered_at.is_none() && self.steer_pending() {
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
        if old_cwd.as_ref() == Some(&new_cwd) {
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

use super::tool_helpers::{build_globset, to_rel_string};
use super::{SearchMatch, ToolResult, Tools};
use anyhow::Result;
use grep::regex::RegexMatcher;
use grep::searcher::sinks::UTF8;
use grep::searcher::Searcher;
use ignore::WalkBuilder;
use serde::Deserialize;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::warn;

#[derive(Debug, Deserialize)]
pub(super) struct SearchArgs {
    pub(super) query: String,
    pub(super) globs: Option<Vec<String>>,
    pub(super) max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RunCommandArgs {
    pub(super) cmd: String,
    pub(super) timeout_ms: Option<u64>,
    #[serde(skip)]
    pub(super) cancel_flag: Option<Arc<AtomicBool>>,
}

fn kill_process_group(child: &std::process::Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        unsafe {
            libc::killpg(pid, libc::SIGTERM);
        }
        std::thread::sleep(Duration::from_millis(100));
        unsafe {
            libc::killpg(pid, libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        // On non-unix, we can't use killpg; the caller already uses child.kill().
        let _ = child;
    }
}

impl Tools {
    pub(super) fn search_rg(&self, args: SearchArgs) -> Result<ToolResult> {
        let globset = build_globset(args.globs.as_deref())?;
        let max_results = args.max_results.unwrap_or(200);

        let matcher = RegexMatcher::new(&args.query).or_else(|_| {
            let escaped = regex::escape(&args.query);
            RegexMatcher::new(&escaped)
                .map_err(|e| anyhow::anyhow!("invalid search pattern '{}': {}", args.query, e))
        })?;

        let mut matches = Vec::new();
        let mut searcher = Searcher::new();
        let walker = WalkBuilder::new(&self.root)
            .standard_filters(true)
            .hidden(true)
            .build();

        for entry in walker {
            let entry = match entry {
                Ok(v) => v,
                Err(_) => continue,
            };
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }

            let path = entry.path();
            let rel_path = to_rel_string(&self.root, path)?;
            if let Some(gs) = &globset {
                if !gs.is_match(Path::new(&rel_path)) {
                    continue;
                }
            }
            let mut file_matches = Vec::new();

            let _ = searcher.search_path(
                &matcher,
                path,
                UTF8(|line_num, line_content| {
                    file_matches.push(SearchMatch {
                        path: rel_path.clone(),
                        line: line_num as usize,
                        snippet: line_content.trim_end().to_string(),
                    });
                    if matches.len() + file_matches.len() >= max_results {
                        return Ok(false);
                    }
                    Ok(true)
                }),
            );

            matches.extend(file_matches);
            if matches.len() >= max_results {
                break;
            }
        }

        Ok(ToolResult::SearchMatches(matches))
    }

    pub(super) fn run_command(&self, args: RunCommandArgs) -> Result<ToolResult> {
        use std::io::BufRead;

        const CWD_SENTINEL: &str = "__LINGGEN_CWD__";

        let timeout = Duration::from_millis(args.timeout_ms.unwrap_or(30000));
        let cwd = self.cwd();

        // Wrap the user command to capture the final working directory.
        // Preserves the original exit code while appending a sentinel + pwd.
        let wrapped_cmd = format!(
            "{}; __linggen_ec=$?; echo '{}'; pwd; exit $__linggen_ec",
            &args.cmd, CWD_SENTINEL
        );

        let mut child = if cfg!(target_os = "windows") {
            Command::new("cmd")
                .args(["/C", &args.cmd])
                .current_dir(&cwd)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?
        } else {
            use std::os::unix::process::CommandExt;
            Command::new("sh")
                .arg("-c")
                .arg(&wrapped_cmd)
                .current_dir(&cwd)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .process_group(0)
                .spawn()?
        };

        // Take stdout/stderr handles for line-by-line streaming.
        let child_stdout = child.stdout.take();
        let child_stderr = child.stderr.take();
        let progress_tx = self.progress_tx.clone();

        // Spawn reader threads that accumulate output and optionally send
        // progress lines through the channel.
        let stdout_handle = std::thread::spawn({
            let tx = progress_tx.clone();
            move || {
                let mut acc = String::new();
                if let Some(stdout) = child_stdout {
                    let reader = std::io::BufReader::new(stdout);
                    for line in reader.lines() {
                        match line {
                            Ok(l) => {
                                if let Some(tx) = &tx {
                                    let _ = tx.send(("Bash".to_string(), "stdout".to_string(), l.clone()));
                                }
                                acc.push_str(&l);
                                acc.push('\n');
                            }
                            Err(_) => break,
                        }
                    }
                }
                acc
            }
        });
        let stderr_handle = std::thread::spawn({
            let tx = progress_tx;
            move || {
                let mut acc = String::new();
                if let Some(stderr) = child_stderr {
                    let reader = std::io::BufReader::new(stderr);
                    for line in reader.lines() {
                        match line {
                            Ok(l) => {
                                if let Some(tx) = &tx {
                                    let _ = tx.send(("Bash".to_string(), "stderr".to_string(), l.clone()));
                                }
                                acc.push_str(&l);
                                acc.push('\n');
                            }
                            Err(_) => break,
                        }
                    }
                }
                acc
            }
        });

        // Wait for the process with timeout.
        let start = Instant::now();
        let mut timed_out = false;
        let mut interrupted = false;
        loop {
            if let Some(_status) = child.try_wait()? {
                break;
            }
            if start.elapsed() >= timeout {
                timed_out = true;
                kill_process_group(&child);
                break;
            }
            if let Some(flag) = &args.cancel_flag {
                if flag.load(std::sync::atomic::Ordering::Relaxed) {
                    interrupted = true;
                    kill_process_group(&child);
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }

        let exit_status = child.wait()?;

        // Join reader threads and collect accumulated output.
        let stdout = stdout_handle.join().unwrap_or_else(|_| {
            warn!("stdout reader thread panicked for command");
            String::new()
        });
        let mut stderr = stderr_handle.join().unwrap_or_else(|_| {
            warn!("stderr reader thread panicked for command");
            "linggen: internal error reading command output\n".to_string()
        });

        // Strip the cwd sentinel from stdout and update persistent cwd.
        let stdout = {
            let mut lines: Vec<&str> = stdout.lines().collect();
            // Look for sentinel from the end (it's always the second-to-last line).
            let sentinel_pos = lines.iter().rposition(|l| *l == CWD_SENTINEL);
            if let Some(pos) = sentinel_pos {
                // The line after the sentinel is the pwd output.
                if pos + 1 < lines.len() {
                    let new_cwd = std::path::PathBuf::from(lines[pos + 1]);
                    if new_cwd.is_absolute() && new_cwd.exists() {
                        if let Some(sid) = &self.session_id {
                            self.cwd_by_session.lock().unwrap().insert(sid.clone(), new_cwd);
                        }
                    }
                }
                // Remove sentinel line and pwd line from output.
                let drain_end = (pos + 2).min(lines.len());
                lines.drain(pos..drain_end);
            }
            let mut cleaned = lines.join("\n");
            // Restore trailing newline if original had one.
            if !cleaned.is_empty() {
                cleaned.push('\n');
            }
            cleaned
        };

        if timed_out {
            if !stderr.is_empty() && !stderr.ends_with('\n') {
                stderr.push('\n');
            }
            stderr.push_str(&format!(
                "linggen: command timed out after {}ms\n",
                timeout.as_millis()
            ));
        }

        if interrupted {
            if !stderr.is_empty() && !stderr.ends_with('\n') {
                stderr.push('\n');
            }
            stderr.push_str("linggen: command interrupted by user\n");
        }

        Ok(ToolResult::CommandOutput {
            exit_code: exit_status.code(),
            stdout,
            stderr,
        })
    }
}

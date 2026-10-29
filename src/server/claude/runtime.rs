use super::protocol::TurnRequest;
use super::{install, resolve_model, state, Control, AGENT_ID};
use crate::engine::agent::AgentEvent;
use crate::server::ServerState;
use crate::state_fs::sessions::SessionMeta;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{path::PathBuf, process::Stdio, sync::Arc, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

pub(super) struct ClaudeSession {
    pub state: Arc<ServerState>,
    pub meta: SessionMeta,
    pub run_id: String,
}

impl ClaudeSession {
    pub async fn emit(&self, mut event: AgentEvent) {
        event.stamp_run(Some(&self.run_id), None);
        self.state
            .manager
            .send_event(event, Some(self.meta.id.clone()))
            .await;
    }

    /// Only a real cancel (Stop, Clear, Delete) ends a turn; a follow-up
    /// message waits for the turn instead.
    pub async fn cancelled(&self) -> bool {
        self.state.manager.is_run_cancelled(&self.run_id).await
    }

    /// Await `work`, giving up promptly once the run is cancelled.
    pub async fn unless_cancelled<T>(
        &self,
        work: impl std::future::Future<Output = T>,
    ) -> Result<T> {
        tokio::pin!(work);
        let mut tick = cancel_tick();
        loop {
            tokio::select! {
                value = &mut work => return Ok(value),
                _ = tick.tick() => if self.cancelled().await { bail!("run cancelled"); }
            }
        }
    }

    pub fn dir(&self) -> PathBuf {
        self.state
            .manager
            .global_sessions
            .session_dir(&self.meta.id)
    }

    /// Start the session's runner on its first turn and serve it until it
    /// ends (doc/background-tasks-spec.md): later turns arrive on `turns`.
    /// A start that fails fails the first turn with it.
    pub async fn serve(
        &mut self,
        first: TurnRequest,
        chosen: Option<String>,
        turns: tokio::sync::mpsc::UnboundedReceiver<TurnRequest>,
        controls: tokio::sync::mpsc::UnboundedReceiver<Control>,
        settings: Arc<std::sync::Mutex<String>>,
    ) -> Result<()> {
        let mut first = Some(first);
        let result = self.start_and_serve(&mut first, chosen, turns, controls, settings).await;
        if let Some(turn) = first {
            let reason = match &result {
                Ok(()) => "Claude Agent SDK did not start".to_string(),
                Err(error) => error.to_string(),
            };
            let _ = turn.done.send(Err(reason));
        }
        result
    }

    async fn start_and_serve(
        &mut self,
        first: &mut Option<TurnRequest>,
        chosen: Option<String>,
        turns: tokio::sync::mpsc::UnboundedReceiver<TurnRequest>,
        controls: tokio::sync::mpsc::UnboundedReceiver<Control>,
        settings: Arc<std::sync::Mutex<String>>,
    ) -> Result<()> {
        if self.cancelled().await {
            bail!("run cancelled");
        }
        // Refresh session settings after a queued turn acquired its lock.
        self.meta = self
            .state
            .manager
            .global_sessions
            .get_session_meta(&self.meta.id)?
            .context("Session was removed")?;
        // A resumed conversation runs in the folder it started in, wherever
        // its shell moved the session since: this runner's folder.
        let resume = state::resume(&self.dir(), &self.meta.cwd).map(|(id, started)| {
            self.meta.cwd = started;
            id
        });
        let model = resolve_model(
            &self.state,
            chosen
                .as_deref()
                .or_else(|| self.meta.member(AGENT_ID).and_then(|m| m.model.as_deref())),
        )
        .await?;
        let runner = self.unless_cancelled(install::ensure_installed()).await??;
        let trusted = self.interaction().folder_trust().await?;
        *settings.lock().unwrap_or_else(|p| p.into_inner()) =
            self.settings_key(&model.id, trusted).await;
        let session_memory = self.session_memory().await;
        let mut command = sdk_command(&runner, &self.meta.cwd, &model)?;
        // The SDK's shells inherit it: `ling restart` knows who asked.
        command.env(crate::paths::SESSION_ENV, &self.meta.id);
        let mut child = command
            .spawn()
            .context("Could not start Claude Agent SDK")?;
        let pid = child.id();
        if let Some(pid) = pid {
            self.state.claude.runner_started(&self.meta.id, pid);
        }
        let mut stdin = child.stdin.take().context("SDK stdin missing")?;
        let mut lines = BufReader::new(child.stdout.take().context("SDK stdout missing")?).lines();
        let stderr = child.stderr.take().context("SDK stderr missing")?;
        let drain = drain_stderr(stderr);
        // Session approvals live in engine memory, like Claude Code's own;
        // a file older builds wrote is not reloaded.
        let _ = std::fs::remove_file(self.dir().join(LEGACY_PERMISSIONS));
        let permission_updates = self.state.claude.session_rules(&self.meta.id);
        let turn = first.take().context("The first turn was already taken")?;
        let start = json!({"type":"start", "prompt":turn.prompt, "images":turn.images, "uuid":turn.uuid,
            "cwd":self.meta.cwd, "model":model.model, "resume":resume,
            "permissionMode":self.meta.claude_permission_mode,
            "permissionUpdates":permission_updates, "settingSources":super::trust::setting_sources(trusted),
            "sessionMemory":session_memory, "turnMemory":turn.turn_memory,
            "disallowedTools":self.meta.withheld_tools, "mcpServers":self.mcp_servers().await});
        // Every post-spawn error goes through cleanup, including a broken stdin.
        let mut stranded = Vec::new();
        let result = async {
            stdin.write_all(format!("{start}\n").as_bytes()).await?;
            self.consume(&mut lines, &mut stdin, turn, turns, controls, &mut stranded).await
        }
        .await;
        let _ = stdin.write_all(b"{\"type\":\"cancel\"}\n").await;
        drop(stdin);
        if tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .is_err()
        {
            #[cfg(unix)]
            if let Some(pid) = pid {
                unsafe {
                    libc::killpg(pid as i32, libc::SIGKILL);
                }
            }
            let _ = child.kill().await;
        }
        let result = with_stderr(result, drain).await;
        if let Some(pid) = pid {
            self.state.claude.runner_ended(&self.meta.id, pid);
        }
        let reason = match &result {
            Ok(()) => "Claude Agent SDK ended before answering".to_string(),
            Err(error) => error.to_string(),
        };
        for (run_id, done) in stranded {
            self.end_stream(&run_id);
            let _ = done.send(Err(reason.clone()));
        }
        result
    }

    pub fn interaction(&self) -> super::interaction::Interaction {
        super::interaction::Interaction {
            agent_id: AGENT_ID.into(),
            state: self.state.clone(),
            meta: self.meta.clone(),
            run_id: self.run_id.clone(),
            question_id: super::interaction::new_question_id(),
        }
    }

    /// Memory as a native owner session has it: the session-start block in
    /// the system prompt, once per runner.
    async fn session_memory(&self) -> String {
        let config = self.state.manager.get_config_snapshot().await;
        let store = self.state.prompt_store.clone();
        let cwd = PathBuf::from(&self.meta.cwd);
        let url = config.agent.ling_mem_url.clone();
        tokio::task::spawn_blocking(move || {
            crate::engine::prompt::core_block::owner_block(&store, &url, &cwd)
        })
        .await
        .unwrap_or_default()
    }

    /// This turn's recall and capture nudge, beside its message. A slash
    /// command goes to the SDK as-is, without recall.
    pub async fn turn_memory(&self, prompt: &str) -> String {
        if prompt.trim_start().starts_with('/') {
            return String::new();
        }
        crate::server::chat::owner_turn_memory(&self.state, AGENT_ID, prompt, &self.meta.id).await
    }

    /// What a runner is started on and cannot change while it lives: the
    /// model, folder trust, MCP servers, withheld tools.
    pub async fn settings_key(&self, model: &str, trusted: bool) -> String {
        json!([model, trusted, self.mcp_servers().await, self.meta.withheld_tools]).to_string()
    }

    async fn mcp_servers(&self) -> Value {
        let config = self.state.manager.get_config_snapshot().await;
        let mut servers = serde_json::Map::new();
        for (name, server) in
            crate::mcp_client::with_builtin(&config.mcp_servers, &config.agent.ling_mem_url)
        {
            if !server.enabled
                || self
                    .meta
                    .withheld_tools
                    .iter()
                    .any(|t| t == &format!("mcp__{name}"))
            {
                continue;
            }
            let transport = match (server.url, server.command) {
                (Some(url), _) => json!({"type":"http", "url":url, "headers":server.headers}),
                (_, Some(command)) => {
                    json!({"type":"stdio", "command":command, "args":server.args, "env":server.env})
                }
                _ => continue,
            };
            servers.insert(name, transport);
        }
        Value::Object(servers)
    }
}

pub(super) const LEGACY_PERMISSIONS: &str = "claude-permissions.json";

/// One cancel check cadence for every wait on a running turn.
pub(super) fn cancel_tick() -> tokio::time::Interval {
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tick
}

/// The engine's environment the SDK may see: what a shell, bun and TLS need.
/// Everything else the engine holds (other providers' keys, tokens) stays out.
const INHERITED_ENV: [&str; 16] = [
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "TMPDIR",
    "LANG",
    "TZ",
    "HTTPS_PROXY",
    "HTTP_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "NODE_EXTRA_CA_CERTS",
];

pub(super) fn inherited_env(vars: impl Iterator<Item = (String, String)>) -> Vec<(String, String)> {
    vars.filter(|(key, _)| {
        let upper = key.to_ascii_uppercase();
        INHERITED_ENV.contains(&upper.as_str()) || upper.starts_with("LC_")
    })
    .collect()
}

/// Where the SDK keeps its own state (`CLAUDE_CONFIG_DIR`), conversations
/// included.
pub(super) fn sdk_state_dir() -> PathBuf {
    crate::paths::linggen_home().join("runtimes/claude-agent-sdk/state")
}

fn sdk_command(
    runner: &std::path::Path,
    cwd: &str,
    model: &crate::config::ModelConfig,
) -> Result<Command> {
    let mut command = Command::new(install::bun_path()?);
    command
        .arg(runner)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .envs(inherited_env(std::env::vars()))
        // The SDK authenticates with this key. Claude Code passes its own
        // environment to the Bash tool; its scrub switch would also force
        // the default permission mode, so the key stays visible there.
        .env(
            "ANTHROPIC_API_KEY",
            model.api_key.as_deref().unwrap_or_default(),
        )
        .env(
            "ANTHROPIC_BASE_URL",
            if model.url.is_empty() {
                "https://api.anthropic.com"
            } else {
                &model.url
            },
        )
        .env("CLAUDE_CONFIG_DIR", sdk_state_dir())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    Ok(command)
}

/// Probe SDK mode availability without a model turn or a saved conversation.
pub(super) async fn validate_mode(
    state: &ServerState,
    meta: &SessionMeta,
    mode: crate::state_fs::sessions::ClaudePermissionMode,
    trusted: bool,
) -> Result<()> {
    let runner = install::ensure_installed().await?;
    let model = resolve_model(
        state,
        meta.member(AGENT_ID).and_then(|m| m.model.as_deref()),
    )
    .await?;
    let mut child = sdk_command(&runner, &meta.cwd, &model)?.spawn()?;
    let mut stdin = child.stdin.take().context("SDK stdin missing")?;
    let mut lines = BufReader::new(child.stdout.take().context("SDK stdout missing")?).lines();
    let stderr = child.stderr.take().context("SDK stderr missing")?;
    let drain = drain_stderr(stderr);
    let request = json!({"type":"start", "cwd":meta.cwd, "model":model.model, "validateOnly":true, "permissionMode":mode,
        "settingSources":super::trust::setting_sources(trusted)});
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        stdin.write_all(format!("{request}\n").as_bytes()).await?;
        while let Some(line) = lines.next_line().await? {
            let event: Value = serde_json::from_str(&line)?;
            if event["type"] == "mode_result" {
                if event["ok"] == true {
                    let dir = state.manager.global_sessions.session_dir(&meta.id);
                    state::save_capabilities(&dir, &event["commands"])?;
                    if mode == crate::state_fs::sessions::ClaudePermissionMode::Auto {
                        state::mark_auto_verified(&dir);
                    }
                    return Ok(());
                }
                bail!("{}", event["error"].as_str().unwrap_or("Mode unavailable"));
            }
            if event["type"] == "error" {
                bail!("{}", event["message"]);
            }
        }
        bail!("Claude exited before validating the permission mode")
    })
    .await
    .context("Claude mode validation timed out")
    .and_then(|result| result);
    let _ = stdin.write_all(b"{\"type\":\"cancel\"}\n").await;
    drop(stdin);
    if tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await
        .is_err()
    {
        #[cfg(unix)]
        if let Some(pid) = child.id() {
            unsafe {
                libc::killpg(pid as i32, libc::SIGKILL);
            }
        }
        let _ = child.kill().await;
    }
    with_stderr(result, drain).await
}

/// Keep the last lines the bridge printed to stderr: the only clue when it
/// dies before speaking the protocol.
fn drain_stderr(stderr: tokio::process::ChildStderr) -> tokio::task::JoinHandle<String> {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut tail = std::collections::VecDeque::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if tail.len() == 8 {
                tail.pop_front();
            }
            tail.push_back(line);
        }
        Vec::from(tail).join("\n")
    })
}

/// Attach the stderr tail to a failure; the child has already been reaped.
async fn with_stderr<T>(result: Result<T>, drain: tokio::task::JoinHandle<String>) -> Result<T> {
    let abort = drain.abort_handle();
    let tail = tokio::time::timeout(Duration::from_millis(500), drain)
        .await
        .ok()
        .and_then(|joined| joined.ok())
        .unwrap_or_default();
    abort.abort();
    match result {
        Err(error) if !tail.trim().is_empty() && !error.to_string().contains("cancelled") => {
            Err(anyhow::anyhow!("{error}\nSDK stderr: {}", tail.trim()))
        }
        other => other,
    }
}

use super::{install, resolve_model, Control, AGENT_ID};
use crate::engine::agent::AgentEvent;
use crate::server::{ServerEvent, ServerState};
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
    pub interrupt: tokio::sync::mpsc::UnboundedReceiver<String>,
}

impl ClaudeSession {
    pub async fn emit(&self, mut event: AgentEvent) {
        event.stamp_run(Some(&self.run_id), None);
        self.state
            .manager
            .send_event(event, Some(self.meta.id.clone()))
            .await;
    }

    pub async fn cancelled(&mut self) -> bool {
        self.state.manager.is_run_cancelled(&self.run_id).await || self.interrupt.try_recv().is_ok()
    }

    pub fn dir(&self) -> PathBuf {
        self.state
            .manager
            .global_sessions
            .session_dir(&self.meta.id)
    }

    pub fn checkpoint(&self) -> PathBuf {
        self.state
            .manager
            .global_sessions
            .session_dir(&self.meta.id)
            .join("claude.json")
    }

    pub async fn run(
        &mut self,
        prompt: String,
        images: Vec<String>,
        chosen: Option<String>,
        controls: tokio::sync::mpsc::UnboundedReceiver<Control>,
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
        let model = resolve_model(
            &self.state,
            chosen
                .as_deref()
                .or_else(|| self.meta.member(AGENT_ID).and_then(|m| m.model.as_deref())),
        )
        .await?;
        self.emit(AgentEvent::AgentStatus {
            agent_id: AGENT_ID.into(),
            status: "thinking".into(),
            detail: Some("Thinking".into()),
            parent_id: None,
            run_id: None,
            parent_run_id: None,
        })
        .await;
        let runner = {
            let install = install::ensure_installed();
            tokio::pin!(install);
            loop {
                tokio::select! {
                    result = &mut install => break result?,
                    _ = tokio::time::sleep(Duration::from_millis(100)) => if self.cancelled().await { bail!("run cancelled"); }
                }
            }
        };
        let checkpoint: Value = std::fs::read(self.checkpoint())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(Value::Null);
        let resume = checkpoint["session_id"].as_str();
        let mut command = sdk_command(&runner, &self.meta.cwd, &model)?;
        let mut child = command
            .spawn()
            .context("Could not start Claude Agent SDK")?;
        let pid = child.id();
        let mut stdin = child.stdin.take().context("SDK stdin missing")?;
        let mut lines = BufReader::new(child.stdout.take().context("SDK stdout missing")?).lines();
        let stderr = child.stderr.take().context("SDK stderr missing")?;
        let drain = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(_)) = lines.next_line().await {}
        });
        let permission_updates: Value =
            std::fs::read(self.checkpoint().with_file_name("claude-permissions.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or(json!([]));
        let start = json!({"type":"start", "prompt":prompt, "images":images, "cwd":self.meta.cwd,
            "model":model.model, "resume":resume, "permissionMode":self.meta.claude_permission_mode,
            "permissionUpdates":permission_updates,
            "disallowedTools":self.meta.withheld_tools, "mcpServers":self.mcp_servers().await});
        // Every post-spawn error goes through cleanup, including a broken stdin.
        let result = async {
            stdin.write_all(format!("{start}\n").as_bytes()).await?;
            self.consume(&mut lines, &mut stdin, controls).await
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
        drain.abort();
        let _ = self.state.events_tx.send(ServerEvent::Token {
            agent_id: AGENT_ID.into(),
            token: String::new(),
            done: true,
            thinking: false,
            session_id: Some(self.meta.id.clone()),
            run_id: Some(self.run_id.clone()),
        });
        result
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
        .env(
            "CLAUDE_CONFIG_DIR",
            crate::paths::linggen_home().join("runtimes/claude-agent-sdk/state"),
        )
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
    let drain = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(_)) = lines.next_line().await {}
    });
    let request = json!({"type":"start", "cwd":meta.cwd, "model":model.model, "validateOnly":true, "permissionMode":mode});
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        stdin.write_all(format!("{request}\n").as_bytes()).await?;
        while let Some(line) = lines.next_line().await? {
            let event: Value = serde_json::from_str(&line)?;
            if event["type"] == "mode_result" {
                if event["ok"] == true {
                    std::fs::write(
                        state
                            .manager
                            .global_sessions
                            .session_dir(&meta.id)
                            .join("claude-capabilities.json"),
                        serde_json::to_vec(&json!({"commands":event["commands"]}))?,
                    )?;
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
    drain.abort();
    result
}

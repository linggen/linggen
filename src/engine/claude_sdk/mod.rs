//! An external agent loop behind the same Linggen session/run interface.
mod install;
mod protocol;
#[cfg(test)]
mod tests;

use super::agent::AgentEvent;
use super::types::ThinkingEvent;
use super::{AgentEngine, AgentOutcome};
use crate::config::ModelConfig;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

pub(crate) fn supports_model(model: &ModelConfig) -> bool {
    model.provider == "anthropic"
        && model.auth_mode.as_deref() != Some("claude_oauth")
        && model.api_key.as_deref().is_some_and(|key| !key.is_empty())
}

impl AgentEngine {
    pub(super) async fn run_claude_sdk(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<AgentOutcome> {
        self.begin_suggestion_turn();
        self.last_assistant_text = None;
        self.session_id = session_id.map(str::to_owned);
        if let Some(sid) = session_id {
            self.load_session_permissions(sid);
        }
        if self.is_cancelled().await {
            bail!("run cancelled");
        }
        let model = self
            .model_manager
            .list_models()
            .into_iter()
            .find(|model| model.id == self.model_id)
            .cloned()
            .context("Selected model is not configured")?;
        if !supports_model(&model) {
            bail!("Coder needs a Claude model with an Anthropic API key. Choose one in Models.");
        }
        self.emit(AgentEvent::AgentStatus {
            agent_id: self.agent_id.clone().unwrap_or_default(),
            status: "working".into(),
            detail: Some("Preparing Claude Agent SDK".into()),
            parent_id: self.parent_agent_id.clone(),
            run_id: None,
            parent_run_id: None,
        })
        .await;
        // Installation is cancellable too; stopping must not leave a Bun
        // installer running or mark a partial installation as complete.
        let runner = {
            let install = install::ensure_installed();
            tokio::pin!(install);
            loop {
                tokio::select! {
                    result = &mut install => break result?,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                        if self.sdk_interrupted().await { bail!("run cancelled"); }
                    }
                }
            }
        };
        let cwd = self.tools.builtins.cwd();
        let resume_file = self.sdk_resume_file();
        let has_prior_turn = self
            .chat_history
            .iter()
            .any(|message| message.role == "assistant");
        let resume = resume_file
            .as_ref()
            .and_then(|path| read_resume(path, &cwd, has_prior_turn, self.sdk_rewrites()));
        let mut command = Command::new(install::bun_path()?);
        command
            .arg(runner)
            .current_dir(&cwd)
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
        let mut child = command
            .spawn()
            .context("Could not start Claude Agent SDK")?;
        let process_group = child.id();
        let mut stdin = child.stdin.take().context("SDK stdin missing")?;
        let mut lines = BufReader::new(child.stdout.take().context("SDK stdout missing")?).lines();
        // Drain stderr concurrently so a noisy SDK cannot deadlock. Do not
        // echo it to the user: child errors may contain environment details.
        let stderr = child.stderr.take().context("SDK stderr missing")?;
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(_)) = lines.next_line().await {}
        });
        let append = self.sdk_append().await;
        let prompt = self.sdk_prompt(resume.is_some())?;
        let mcp_servers = self.sdk_mcp_servers().await;
        let start = json!({ "type": "start", "prompt": prompt, "images": std::mem::take(&mut self.pending_images),
            "cwd": cwd, "model": model.model, "resume": resume, "append": append,
            "planMode": self.plan_mode, "maxTurns": self.cfg.max_iters,
            "tools": self.spec.as_ref().map(|spec| &spec.tools),
            "mcpServers": mcp_servers });
        stdin.write_all(format!("{start}\n").as_bytes()).await?;
        let result = self
            .consume_sdk(&mut lines, &mut stdin, resume_file.as_deref(), &cwd)
            .await;
        // Abort via the SDK first, allowing its subprocess to save transcripts
        // and unwind hooks, then enforce a bounded shutdown.
        let _ = stdin.write_all(b"{\"type\":\"cancel\"}\n").await;
        drop(stdin);
        if tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
            .await
            .is_err()
        {
            #[cfg(unix)]
            if let Some(pid) = process_group {
                unsafe {
                    libc::killpg(pid as i32, libc::SIGKILL);
                }
            }
            let _ = child.kill().await;
        }
        stderr_task.abort();
        self.stream(ThinkingEvent::ContentDone);
        result
    }

    async fn sdk_interrupted(&mut self) -> bool {
        self.is_cancelled().await
            || self
                .interrupt_rx
                .as_mut()
                .is_some_and(|rx| rx.try_recv().is_ok())
    }

    fn sdk_resume_file(&self) -> Option<PathBuf> {
        // A delegated run has its own ephemeral context, never its parent's
        // SDK conversation. Its SDK transcript must not become a future resume.
        if self.tools.builtins.delegation_depth() > 0 {
            return None;
        }
        self.session_dir.as_ref().map(|dir| {
            dir.join(format!(
                "sdk-{}.json",
                self.agent_id.as_deref().unwrap_or("agent")
            ))
        })
    }

    fn sdk_rewrites(&self) -> u64 {
        match (self.tools.get_manager(), self.session_id.as_deref()) {
            (Some(manager), Some(sid)) => manager.global_sessions.rewrite_count(sid),
            _ => 0,
        }
    }

    async fn sdk_mcp_servers(&self) -> Value {
        let configured = if let Some(manager) = self.tools.get_manager() {
            manager.get_config_snapshot().await.mcp_servers
        } else {
            Default::default()
        };
        let mut servers = serde_json::Map::new();
        for (name, server) in crate::mcp_client::with_builtin(&configured, &self.cfg.ling_mem_url) {
            if !server.enabled {
                continue;
            }
            let transport = match (server.url, server.command) {
                (Some(url), _) => json!({ "type": "http", "url": url, "headers": server.headers }),
                (_, Some(command)) => {
                    json!({ "type": "stdio", "command": command, "args": server.args, "env": server.env })
                }
                _ => continue,
            };
            servers.insert(name, transport);
        }
        Value::Object(servers)
    }

    fn sdk_prompt(&self, resumed: bool) -> Result<String> {
        let task = self.task.as_deref().context("No task set")?;
        let start = if resumed {
            self.chat_history
                .iter()
                .rposition(|message| message.role == "assistant")
                .map_or(0, |index| index + 1)
        } else {
            0
        };
        let context: Vec<Value> = self.chat_history[start..]
            .iter()
            .filter(|message| {
                matches!(message.role.as_str(), "user" | "assistant" | "system")
                    && !message.content.is_empty()
            })
            .map(|message| json!({ "role": message.role, "content": message.content }))
            .collect();
        if context.is_empty() {
            return Ok(task.to_owned());
        }
        Ok(format!(
            "Linggen conversation context (chronological):\n{}\n\nCurrent task:\n{task}",
            serde_json::to_string(&context)?
        ))
    }

    async fn sdk_append(&self) -> String {
        let mut append = self.spec_system_prompt.clone().unwrap_or_default();
        append.push_str("\nYou work inside Linggen. Follow project CLAUDE.md and AGENTS.md instructions before editing. Linggen handles tool permissions and user questions. Use ling-mem MCP for shared memory.\n");
        append.push_str(&format!(
            "Reply in {} unless the person asks otherwise.\n",
            crate::language::display_name(&crate::language::effective())
        ));
        if self.prompt_profile.include_memory {
            let url = self.cfg.ling_mem_url.clone();
            let place = super::tools::place_of(&self.tools.builtins.cwd());
            let core = tokio::task::spawn_blocking(move || {
                super::prompt::core_block::load_session_start(&url, place.as_ref(), true)
            })
            .await;
            if let Ok(Some(core)) = core {
                append.push_str(&self.prompt_store.render_or_fallback(
                    crate::prompts::keys::CORE_MEMORY_BLOCK,
                    &[("core_facts", &core.facts)],
                ));
            }
        }
        if let Some(skill) = &self.active_skill {
            append.push_str(&skill.content);
        }
        if let Some(mission) = &self.active_mission {
            append.push_str(&mission.body);
        }
        append
    }
}

fn read_resume(path: &Path, cwd: &Path, has_history: bool, rewrites: u64) -> Option<String> {
    if !has_history {
        return None;
    }
    let data: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    if data["rewrites"].as_u64().unwrap_or(0) != rewrites {
        return None;
    }
    (data["cwd"].as_str()? == cwd.to_string_lossy())
        .then(|| data["session_id"].as_str().map(str::to_owned))
        .flatten()
}

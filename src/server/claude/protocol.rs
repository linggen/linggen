use super::{runtime::ClaudeSession, AGENT_ID};
use crate::engine::agent::AgentEvent;
use crate::server::ServerEvent;
use crate::state_fs::sessions::ChatMsg;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::io::{AsyncWriteExt, BufReader, Lines};
use tokio::process::{ChildStdin, ChildStdout};

impl ClaudeSession {
    pub async fn consume(
        &mut self,
        lines: &mut Lines<BufReader<ChildStdout>>,
        stdin: &mut ChildStdin,
    ) -> Result<()> {
        let mut active = HashMap::new();
        let result = self.read_events(lines, stdin, &mut active).await;
        for (id, tool) in active {
            self.tool_end(&id, &tool, true, &json!("Task stopped"))
                .await;
        }
        result
    }

    async fn read_events(
        &mut self,
        lines: &mut Lines<BufReader<ChildStdout>>,
        stdin: &mut ChildStdin,
        active: &mut HashMap<String, String>,
    ) -> Result<()> {
        let mut has_text = false;
        loop {
            let line = tokio::select! {
                line = lines.next_line() => line?.context("Claude Agent SDK exited without a result")?,
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                    if self.cancelled().await { bail!("run cancelled"); } continue;
                }
            };
            let event: Value = serde_json::from_str(&line).context("Invalid SDK bridge message")?;
            match event["type"].as_str().unwrap_or_default() {
                "init" => {
                    std::fs::write(
                        self.checkpoint(),
                        serde_json::to_vec(
                            &json!({"session_id":event["session_id"], "cwd":self.meta.cwd}),
                        )?,
                    )?;
                }
                "token" => {
                    if let Some(text) = event["text"].as_str() {
                        let _ = self.state.events_tx.send(ServerEvent::Token {
                            agent_id: AGENT_ID.into(),
                            token: text.into(),
                            done: false,
                            thinking: false,
                            session_id: Some(self.meta.id.clone()),
                            run_id: Some(self.run_id.clone()),
                        });
                    }
                }
                "status" => {
                    if let Ok(mode) = serde_json::from_value(event["permissionMode"].clone()) {
                        if let Some(mut meta) = self
                            .state
                            .manager
                            .global_sessions
                            .get_session_meta(&self.meta.id)?
                        {
                            meta.claude_permission_mode = mode;
                            self.state
                                .manager
                                .global_sessions
                                .update_session_meta(&meta)?;
                        }
                        self.emit(AgentEvent::StateUpdated).await;
                    }
                    if event["status"] == "compacting" {
                        self.emit(AgentEvent::AgentStatus {
                            agent_id: AGENT_ID.into(),
                            status: "thinking".into(),
                            detail: Some("Thinking (compacting context)".into()),
                            parent_id: None,
                            run_id: None,
                            parent_run_id: None,
                        })
                        .await;
                    }
                }
                "text" => {
                    if let Some(text) = event["text"].as_str().filter(|s| !s.is_empty()) {
                        self.text(text).await;
                        has_text = true;
                    }
                }
                "tool_start" => {
                    let id = event["id"].as_str().context("SDK tool has no id")?;
                    let tool = event["tool"].as_str().context("SDK tool has no name")?;
                    let args =
                        crate::engine::render::sanitize_tool_args_for_display(tool, &event["args"]);
                    self.emit(AgentEvent::ContentBlockStart {
                        agent_id: AGENT_ID.into(),
                        block_id: id.into(),
                        block_type: "tool_use".into(),
                        tool: Some(tool.into()),
                        args: Some(args.to_string()),
                        parent_id: None,
                        run_id: None,
                        parent_run_id: None,
                    })
                    .await;
                    self.save(
                        json!({"type":"tool", "tool":tool, "args":args}).to_string(),
                        true,
                        false,
                    );
                    active.insert(id.into(), tool.into());
                }
                "tool_end" => {
                    let id = event["id"].as_str().unwrap_or_default();
                    if let Some(tool) = active.remove(id) {
                        self.tool_end(
                            id,
                            &tool,
                            event["failed"].as_bool().unwrap_or(false),
                            &event["result"],
                        )
                        .await;
                    }
                }
                "request" => {
                    let response = self.request(&event).await?;
                    let mut reply = response.as_object().cloned().unwrap_or_default();
                    reply.insert("type".into(), json!("reply"));
                    reply.insert("id".into(), event["id"].clone());
                    stdin
                        .write_all(format!("{}\n", Value::Object(reply)).as_bytes())
                        .await?;
                }
                "result" => {
                    if !event["success"].as_bool().unwrap_or(false) {
                        bail!("Claude Agent SDK: {}", event["errors"]);
                    }
                    if !has_text {
                        if let Some(text) = event["text"].as_str().filter(|s| !s.is_empty()) {
                            self.text(text).await;
                        }
                    }
                    let count = |key: &str| event["usage"][key].as_u64().unwrap_or(0) as usize;
                    let input = count("input_tokens")
                        + count("cache_creation_input_tokens")
                        + count("cache_read_input_tokens");
                    self.emit(AgentEvent::ContextUsage {
                        agent_id: AGENT_ID.into(),
                        stage: "sdk".into(),
                        message_count: 0,
                        char_count: 0,
                        estimated_tokens: input,
                        token_limit: None,
                        actual_prompt_tokens: Some(input),
                        actual_completion_tokens: Some(count("output_tokens")),
                        compressed: false,
                        summary_count: 0,
                    })
                    .await;
                    return Ok(());
                }
                "error" => bail!(
                    "Claude Agent SDK: {}",
                    event["message"].as_str().unwrap_or("runtime error")
                ),
                _ => bail!("Unknown SDK bridge message"),
            }
        }
    }

    pub async fn text(&self, text: &str) {
        self.save(text.into(), false, false);
        self.emit(AgentEvent::TextSegment {
            agent_id: AGENT_ID.into(),
            text: text.into(),
            parent_id: None,
            run_id: None,
        })
        .await;
    }

    fn save(&self, content: String, observation: bool, failed: bool) {
        let row = ChatMsg {
            run_id: Some(self.run_id.clone()),
            agent_id: AGENT_ID.into(),
            from_id: AGENT_ID.into(),
            to_id: "user".into(),
            content,
            timestamp: crate::util::now_ts_secs(),
            is_observation: observation,
            failed,
            client_id: None,
            via: None,
            via_note: None,
            images: Vec::new(),
        };
        if let Err(error) = self
            .state
            .manager
            .global_sessions
            .add_chat_message(&self.meta.id, &row)
        {
            tracing::error!("Could not save Claude event: {error}");
        }
    }

    async fn tool_end(&self, id: &str, tool: &str, failed: bool, result: &Value) {
        let rendered: String = result
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| result.to_string())
            .chars()
            .take(16_384)
            .collect();
        self.emit(AgentEvent::ContentBlockUpdate {
            agent_id: AGENT_ID.into(),
            block_id: id.into(),
            status: Some(if failed { "failed" } else { "done" }.into()),
            summary: Some(rendered.clone()),
            is_error: Some(failed),
            parent_id: None,
            extra: None,
            run_id: None,
            parent_run_id: None,
        })
        .await;
        self.save(format!("Tool {tool}: {rendered}"), true, failed);
    }
}

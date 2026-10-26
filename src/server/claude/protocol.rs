use super::{runtime::ClaudeSession, state, AGENT_ID};
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
        controls: tokio::sync::mpsc::UnboundedReceiver<super::Control>,
    ) -> Result<()> {
        let mut active = HashMap::new();
        let mut subagents = super::subagents::Subagents::default();
        let result = self
            .read_events(lines, stdin, &mut active, controls, &mut subagents)
            .await;
        if let Err(error) = subagents.finish(self).await {
            tracing::warn!("Could not finish SDK child views: {error}");
        }
        let ids: Vec<String> = self
            .state
            .pending_ask_user
            .lock()
            .await
            .iter()
            .filter(|(_, item)| {
                item.session_id.as_deref() == Some(&self.meta.id)
                    && (item.agent_id == AGENT_ID
                        || item
                            .agent_id
                            .starts_with(&format!("sdk-sub-{}-", self.run_id)))
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.dismiss_question(&id).await;
        }
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
        mut controls: tokio::sync::mpsc::UnboundedReceiver<super::Control>,
        subagents: &mut super::subagents::Subagents,
    ) -> Result<()> {
        let mut has_text = false;
        let mut last_text = String::new();
        let mut requests = tokio::task::JoinSet::new();
        let mut request_tasks = HashMap::new();
        let mut questions: HashMap<i64, String> = HashMap::new();
        let mut changes = HashMap::new();
        let mut next_change = 0_i64;
        let mut controls_open = true;
        // One clock for the whole turn: a sleep made fresh each pass would
        // never fire while tokens stream, and Stop would wait for a pause.
        let mut tick = super::runtime::cancel_tick();
        loop {
            let line = tokio::select! {
                response = requests.join_next(), if !requests.is_empty() => {
                    if let Some(Ok((id, response))) = response {
                        request_tasks.remove(&id);
                        questions.remove(&id);
                        let response: Result<Value> = response;
                        let mut reply = response.unwrap_or_else(|error| json!({"allow":false,"error":error.to_string(),"reason":error.to_string()}));
                        reply["type"] = json!("reply"); reply["id"] = json!(id);
                        stdin.write_all(format!("{reply}\n").as_bytes()).await?;
                    }
                    continue;
                }
                change = controls.recv(), if controls_open => {
                    if let Some(change) = change {
                        next_change += 1;
                        let (message, reply) = match change {
                            super::Control::Mode(change) => (json!({"type":"mode", "id":next_change, "mode":change.mode}), change.reply),
                            super::Control::StopTask { task_id, reply } => (json!({"type":"stop_task", "id":next_change, "task_id":task_id}), reply),
                        };
                        changes.insert(next_change, reply);
                        stdin.write_all(format!("{message}\n").as_bytes()).await?;
                    } else { controls_open = false; }
                    continue;
                }
                line = lines.next_line() => line?.context("Claude Agent SDK exited without a result")?,
                _ = tick.tick() => {
                    if self.cancelled().await { bail!("run cancelled"); } continue;
                }
            };
            let event: Value = serde_json::from_str(&line).context("Invalid SDK bridge message")?;
            if subagents.handle(self, &event).await? {
                continue;
            }
            match event["type"].as_str().unwrap_or_default() {
                "init" => {
                    state::update_checkpoint(&self.dir(), |checkpoint| {
                        checkpoint["session_id"] = event["session_id"].clone();
                        checkpoint["cwd"] = json!(self.meta.cwd);
                        state::set_commands(checkpoint, &event["commands"]);
                    })?;
                    self.sync_mode(&event).await?;
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
                    self.sync_mode(&event).await?;
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
                "commands" => {
                    state::update_checkpoint(&self.dir(), |checkpoint| {
                        state::set_commands(checkpoint, &event["commands"]);
                    })?;
                }
                "text" => {
                    if let Some(text) = event["text"].as_str().filter(|s| !s.is_empty()) {
                        self.text(text).await;
                        last_text = text.into();
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
                        AGENT_ID,
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
                    let id = event["id"].as_i64().context("SDK request has no id")?;
                    let question_id = super::interaction::new_question_id();
                    questions.insert(id, question_id.clone());
                    let interaction = super::interaction::Interaction {
                        agent_id: event["input"]["agent_id"]
                            .as_str()
                            .map(|id| super::subagents::tracking_id(&self.run_id, id))
                            .unwrap_or_else(|| AGENT_ID.into()),
                        state: self.state.clone(),
                        meta: self.meta.clone(),
                        run_id: self.run_id.clone(),
                        question_id,
                    };
                    let task =
                        requests.spawn(async move { (id, interaction.request(&event).await) });
                    request_tasks.insert(id, task);
                }
                "cancel_request" => {
                    let id = event["id"].as_i64().unwrap_or_default();
                    if let Some(task) = request_tasks.remove(&id) {
                        task.abort();
                    }
                    if let Some(question) = questions.remove(&id) {
                        self.dismiss_question(&question).await;
                    }
                }
                "mode_result" => {
                    if let Some(change) = changes.remove(&event["id"].as_i64().unwrap_or_default())
                    {
                        let _ = change.send(if event["ok"] == true {
                            Ok(())
                        } else {
                            Err(event["error"].as_str().unwrap_or("Mode unavailable").into())
                        });
                    }
                }
                "reset" => {
                    super::usage::clear(&self.dir())?;
                    let _ = std::fs::remove_file(self.dir().join("claude-subagents.json"));
                    state::update_checkpoint(&self.dir(), |checkpoint| {
                        checkpoint["session_id"] = event["session_id"].clone();
                    })?;
                    self.state.claude.forget_session(&self.meta.id);
                    self.state
                        .manager
                        .global_sessions
                        .clear_chat_history(&self.meta.id)?;
                    let _ = self.state.events_tx.send(ServerEvent::SessionReset {
                        session_id: self.meta.id.clone(),
                    });
                    has_text = false;
                    last_text.clear();
                }
                "result" => {
                    super::usage::record(&self.dir(), &event)?;
                    if !event["success"].as_bool().unwrap_or(false) {
                        bail!("Claude Agent SDK: {}", event["errors"]);
                    }
                    if !has_text {
                        if let Some(text) = event["text"].as_str().filter(|s| !s.is_empty()) {
                            self.text(text).await;
                            last_text = text.into();
                        }
                    }
                    if !last_text.is_empty() {
                        self.finalize_text(&last_text).await;
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
                }
                "done" => return Ok(()),
                "error" => bail!(
                    "Claude Agent SDK: {}",
                    event["message"].as_str().unwrap_or("runtime error")
                ),
                _ => bail!("Unknown SDK bridge message"),
            }
        }
    }

    async fn sync_mode(&mut self, event: &Value) -> Result<()> {
        if let Ok(mode) = serde_json::from_value(event["permissionMode"].clone()) {
            if let Some(mut meta) = self
                .state
                .manager
                .global_sessions
                .get_session_meta(&self.meta.id)?
            {
                meta.claude_permission_mode = mode;
                self.meta.claude_permission_mode = mode;
                if mode == crate::state_fs::sessions::ClaudePermissionMode::Auto {
                    state::mark_auto_verified(&self.dir());
                }
                self.state
                    .manager
                    .global_sessions
                    .update_session_meta(&meta)?;
            }
            self.emit(AgentEvent::StateUpdated).await;
        }
        Ok(())
    }

    async fn dismiss_question(&self, id: &str) {
        if self
            .state
            .pending_ask_user
            .lock()
            .await
            .remove(id)
            .is_some()
        {
            let _ = self.state.events_tx.send(ServerEvent::WidgetResolved {
                widget_id: id.into(),
                session_id: Some(self.meta.id.clone()),
            });
        }
    }

    pub async fn text(&self, text: &str) {
        self.save(AGENT_ID, text.into(), false, false);
        self.emit(AgentEvent::TextSegment {
            agent_id: AGENT_ID.into(),
            text: text.into(),
            parent_id: None,
            run_id: None,
        })
        .await;
    }

    pub async fn finalize_text(&self, text: &str) {
        let _ = self.state.events_tx.send(ServerEvent::Message {
            from: AGENT_ID.into(),
            to: "user".into(),
            content: text.into(),
            session_id: Some(self.meta.id.clone()),
            run_id: Some(self.run_id.clone()),
            parent_agent_id: None,
            client_id: None,
            via: None,
            via_note: None,
            images: Vec::new(),
        });
    }

    fn save(&self, from: &str, content: String, observation: bool, failed: bool) {
        let row = ChatMsg {
            run_id: Some(self.run_id.clone()),
            agent_id: AGENT_ID.into(),
            from_id: from.into(),
            to_id: if from == "system" { AGENT_ID } else { "user" }.into(),
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
        self.save("system", format!("Tool {tool}: {rendered}"), true, failed);
    }
}

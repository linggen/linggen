use super::*;
use crate::engine::tools::{ToolCall, ToolResult};
use crate::state_fs::sessions::ChatMsg;
use std::collections::HashMap;
use tokio::io::Lines;
use tokio::process::{ChildStdin, ChildStdout};

impl AgentEngine {
    pub(super) async fn consume_sdk(
        &mut self,
        lines: &mut Lines<BufReader<ChildStdout>>,
        stdin: &mut ChildStdin,
        resume_file: Option<&Path>,
        cwd: &Path,
    ) -> Result<AgentOutcome> {
        let mut active = HashMap::<String, String>::new();
        let result = self
            .read_sdk_events(lines, stdin, resume_file, cwd, &mut active)
            .await;
        // Denied calls, a crash and cancellation all close their live blocks.
        for (id, tool) in active {
            self.sdk_tool_end(&id, &tool, true, &json!("Task stopped"))
                .await;
        }
        result
    }

    async fn read_sdk_events(
        &mut self,
        lines: &mut Lines<BufReader<ChildStdout>>,
        stdin: &mut ChildStdin,
        resume_file: Option<&Path>,
        cwd: &Path,
        active: &mut HashMap<String, String>,
    ) -> Result<AgentOutcome> {
        let mut has_text = false;
        loop {
            let line = tokio::select! {
                line = lines.next_line() => line?.context("Claude Agent SDK exited without a result")?,
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                    if self.sdk_interrupted().await { bail!("run cancelled"); }
                    continue;
                }
            };
            let event: Value = serde_json::from_str(&line).context("Invalid SDK bridge message")?;
            match event["type"].as_str().unwrap_or_default() {
                "init" => {
                    if let Some(path) = resume_file {
                        tokio::fs::write(
                            path,
                            serde_json::to_vec(&json!({
                            "session_id": event["session_id"], "cwd": cwd, "rewrites": self.sdk_rewrites(),
                            }))?,
                        )
                        .await?;
                    }
                }
                "token" => {
                    if let Some(text) = event["text"].as_str() {
                        self.stream(ThinkingEvent::ContentToken(text.into()));
                    }
                }
                "text" => {
                    if let Some(text) = event["text"].as_str().filter(|text| !text.is_empty()) {
                        self.emit_text_segment(text.into(), self.session_id.as_deref())
                            .await;
                        self.chat_history
                            .push(crate::message::ChatMessage::new("assistant", text));
                        self.last_assistant_text = Some(text.into());
                        has_text = true;
                    }
                }
                "tool_start" => {
                    let id = event["id"].as_str().context("SDK tool has no id")?;
                    let tool = event["tool"].as_str().context("SDK tool has no name")?;
                    self.sdk_tool_start(id, tool, &event["args"]).await;
                    active.insert(id.into(), tool.into());
                }
                "tool_end" => {
                    let id = event["id"].as_str().unwrap_or_default();
                    if let Some(tool) = active.remove(id) {
                        self.sdk_tool_end(
                            id,
                            &tool,
                            event["failed"].as_bool().unwrap_or(false),
                            &event["result"],
                        )
                        .await;
                    }
                }
                "request" => {
                    let response = self.sdk_request(&event, cwd).await?;
                    let reply = json!({ "type": "reply", "id": event["id"],
                        "allow": response["allow"], "reason": response["reason"], "answers": response["answers"], "updatedInput": response["updatedInput"] });
                    stdin.write_all(format!("{reply}\n").as_bytes()).await?;
                }
                "result" => {
                    if event["success"].as_bool() != Some(true) {
                        bail!("Claude Agent SDK: {}", event["errors"]);
                    }
                    if !has_text {
                        if let Some(text) = event["text"].as_str() {
                            self.persist_assistant_message(text, self.session_id.as_deref())
                                .await?;
                        }
                    }
                    tracing::info!(agent = ?self.agent_id, cost = ?event["cost"], "Claude Agent SDK completed");
                    let count = |key: &str| event["usage"][key].as_u64().unwrap_or(0) as usize;
                    let prompt = count("input_tokens")
                        + count("cache_creation_input_tokens")
                        + count("cache_read_input_tokens");
                    let output = count("output_tokens");
                    let usage = crate::provider::models::TokenUsage {
                        prompt_tokens: Some(prompt),
                        completion_tokens: Some(output),
                        total_tokens: Some(prompt + output),
                        cached_tokens: Some(count("cache_read_input_tokens")),
                    };
                    self.run_usage.add(&self.model_id, Some(&usage));
                    return Ok(AgentOutcome::None);
                }
                "error" => bail!(
                    "Claude Agent SDK: {}",
                    event["message"].as_str().unwrap_or("runtime error")
                ),
                _ => bail!("Unknown SDK bridge message"),
            }
        }
    }

    async fn sdk_request(&mut self, event: &Value, default_cwd: &Path) -> Result<Value> {
        let input = &event["input"];
        match event["kind"].as_str() {
            Some("permission") => {
                let tool = input["tool_name"]
                    .as_str()
                    .context("SDK permission has no tool")?;
                let cwd = input["cwd"].as_str().map(Path::new).unwrap_or(default_cwd);
                if let Err(reason) = self
                    .authorize_sdk_tool(tool, &input["tool_input"], cwd)
                    .await
                {
                    return Ok(json!({ "allow": false, "reason": reason }));
                }
                let args = match super::super::tools::augment_mcp_args(
                    &self.tools.builtins,
                    tool,
                    input["tool_input"].clone(),
                )
                .await
                {
                    Ok(args) => args,
                    Err(error) => {
                        return Ok(json!({ "allow": false, "reason": error.to_string() }))
                    }
                };
                Ok(json!({ "allow": true, "updatedInput": args }))
            }
            Some("ask_user") => {
                if !self.session_permissions.interactive {
                    return Ok(json!({ "reason": "This task cannot ask questions" }));
                }
                let mut args = input.clone();
                if let Some(questions) = args["questions"].as_array_mut() {
                    for question in questions {
                        question["multi_select"] =
                            json!(question["multiSelect"].as_bool().unwrap_or(false));
                    }
                }
                let result = self
                    .tools
                    .execute(ToolCall {
                        tool: "AskUser".into(),
                        args,
                        block_id: None,
                    })
                    .await?;
                let ToolResult::AskUserResponse { answers } = result else {
                    return Ok(json!({ "reason": "Question cancelled" }));
                };
                let mut mapped = serde_json::Map::new();
                for answer in answers {
                    let Some(question) =
                        input["questions"][answer.question_index]["question"].as_str()
                    else {
                        continue;
                    };
                    let text = answer
                        .custom_text
                        .filter(|text| !text.is_empty())
                        .unwrap_or_else(|| answer.selected.join(", "));
                    mapped.insert(question.into(), json!(text));
                }
                Ok(json!({ "answers": mapped }))
            }
            Some("exit_plan") => {
                if !self.session_permissions.interactive {
                    return Ok(json!({ "allow": false }));
                }
                let question = crate::engine::tools::AskUserQuestion {
                    question: "Approve this plan and begin implementation?".into(),
                    header: "Plan".into(),
                    multi_select: false,
                    options: vec![
                        crate::engine::tools::AskUserOption {
                            label: "Approve".into(),
                            description: None,
                            preview: None,
                        },
                        crate::engine::tools::AskUserOption {
                            label: "Cancel".into(),
                            description: None,
                            preview: None,
                        },
                    ],
                };
                let decision = self.ask_permission_raw("ExitPlanMode", question).await;
                let allow = matches!(
                    decision,
                    Some(crate::engine::permission::PermissionAction::AllowOnce)
                );
                Ok(json!({ "allow": allow }))
            }
            _ => bail!("Unknown SDK request"),
        }
    }

    async fn sdk_tool_start(&self, id: &str, tool: &str, args: &Value) {
        let args = crate::engine::render::sanitize_tool_args_for_display(tool, args);
        let agent = self.agent_id.clone().unwrap_or_default();
        self.emit(AgentEvent::ContentBlockStart {
            agent_id: agent.clone(),
            block_id: id.into(),
            block_type: "tool_use".into(),
            tool: Some(tool.into()),
            args: Some(args.to_string()),
            parent_id: self.parent_agent_id.clone(),
            run_id: None,
            parent_run_id: None,
        })
        .await;
        if self.tools.builtins.delegation_depth() == 0 {
            self.save_row(
                self.session_id.as_deref(),
                ChatMsg {
                    run_id: None,
                    via: None,
                    via_note: None,
                    agent_id: agent.clone(),
                    from_id: agent,
                    to_id: self.outbound_target(),
                    content: json!({ "type": "tool", "tool": tool, "args": args }).to_string(),
                    timestamp: crate::util::now_ts_secs(),
                    is_observation: true,
                    client_id: None,
                    failed: false,
                    images: Vec::new(),
                },
            )
            .await;
        }
    }

    async fn sdk_tool_end(&self, id: &str, tool: &str, failed: bool, result: &Value) {
        let rendered = match result.as_str() {
            Some(text) => text.to_owned(),
            None => result.to_string(),
        };
        let rendered: String = rendered.chars().take(16_384).collect();
        self.emit(AgentEvent::ContentBlockUpdate {
            agent_id: self.agent_id.clone().unwrap_or_default(),
            block_id: id.into(),
            status: Some(if failed { "failed" } else { "done" }.into()),
            summary: Some(rendered.clone()),
            is_error: Some(failed),
            parent_id: self.parent_agent_id.clone(),
            extra: None,
            run_id: None,
            parent_run_id: None,
        })
        .await;
        let _ = self
            .persist_observation(tool, &rendered, failed, self.session_id.as_deref())
            .await;
    }
}

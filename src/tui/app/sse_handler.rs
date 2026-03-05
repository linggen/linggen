use super::super::display::*;
use super::utils::{parse_activity_text, parse_content_block_args};
use super::{ActiveToolGroup, App, ConnectionStatus, InteractivePrompt};
use crate::server::UiSseMessage;
use std::time::Instant;

impl App {
    pub fn handle_sse(&mut self, msg: UiSseMessage) {
        // Seq-based dedup: skip events we've already processed.
        // Connection events (synthetic, seq=0) are always allowed through.
        if msg.kind != "connection" && msg.seq > 0 && msg.seq <= self.last_seq {
            return;
        }
        if msg.seq > 0 {
            self.last_seq = msg.seq;
        }

        match msg.kind.as_str() {
            "token" => {
                let text = msg.text.unwrap_or_default();
                let done = msg.phase.as_deref() == Some("done");
                let is_thinking = msg
                    .data
                    .as_ref()
                    .and_then(|d| d.get("thinking"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                if is_thinking {
                    self.is_thinking_phase = true;
                    // Don't accumulate thinking text in streaming_buffer
                } else if !text.is_empty() {
                    self.is_thinking_phase = false;
                    self.streaming_buffer.push_str(&text);
                    self.is_streaming = true;
                }
                if done {
                    if is_thinking {
                        self.is_thinking_phase = false;
                        self.discard_streaming();
                    } else {
                        self.finalize_streaming();
                    }
                }
            }
            "text_segment" => {
                let agent_id = msg.agent_id.unwrap_or_default();
                let text = msg.text.unwrap_or_default();
                if text.is_empty() {
                    return;
                }
                // Skip subagent text segments
                if self.subagent_parent_map.contains_key(&agent_id.to_lowercase()) {
                    return;
                }
                // Filter internal/status messages
                if Self::should_hide_internal_message(&text) || Self::is_status_line_text(&text) {
                    return;
                }
                // Dedup: skip if this text was already pushed via token streaming
                if self.is_duplicate_agent_text(&text) {
                    self.discard_streaming();
                    return;
                }
                // Discard any thinking tokens being streamed — text_segment is more reliable
                self.discard_streaming();
                // Finalize any active tool group first → creates interleaving
                self.finalize_tool_group();
                self.push_agent(&agent_id, &text);
            }
            "message" => {
                self.discard_streaming();

                let role = msg
                    .data
                    .as_ref()
                    .and_then(|d| d.get("role"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("assistant");
                if role == "user" {
                    if let Some(text) = &msg.text {
                        if let Some(front) = self.pending_user_messages.front() {
                            if front == text {
                                self.pending_user_messages.pop_front();
                                return;
                            }
                        }
                        if !text.is_empty() {
                            self.push_user(text);
                        }
                    }
                    return;
                }
                // Always finalize any active tool/subagent group when a message
                // event arrives, even if the text is stripped — this ensures the
                // tool group moves from "active" to "collapsed" display.
                self.finalize_tool_group();
                self.finalize_subagent_group();
                if let Some(text) = msg.text {
                    let cleaned = Self::strip_internal_json(&text);
                    if !cleaned.is_empty()
                        && !Self::should_hide_internal_message(&cleaned)
                        && !Self::is_status_line_text(&cleaned)
                        && !self.is_duplicate_agent_text(&cleaned)
                    {
                        let agent = msg.agent_id.unwrap_or_default();
                        self.push_agent(&agent, &cleaned);
                    }
                }
            }
            "activity" => {
                self.handle_sse_activity(msg);
            }
            "run" => {
                self.handle_sse_run(msg);
            }
            "ask_user" => {
                if let Some(data) = &msg.data {
                    let question_id = data
                        .get("question_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let questions = data.get("questions").and_then(|v| v.as_array());
                    if let Some(questions) = questions {
                        let header = questions
                            .first()
                            .and_then(|q| q.get("header"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        // Display question text (with "Permission: " prefix for permission prompts)
                        let question_text = questions
                            .first()
                            .and_then(|q| q.get("question"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("Question");
                        if header == "Permission" {
                            self.push_system(&format!("Permission: {}", question_text));
                        } else {
                            self.push_system(question_text);
                        }
                        // Extract option labels and show as InteractivePrompt
                        let options: Vec<String> = questions
                            .first()
                            .and_then(|q| q.get("options"))
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|o| {
                                        o.get("label").and_then(|v| v.as_str()).map(String::from)
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        if !options.is_empty() {
                            self.pending_ask_user_id = Some(question_id);
                            self.prompt = Some(InteractivePrompt {
                                options,
                                selected: 0,
                            });
                        }
                    }
                }
            }
            "model_fallback" => {
                let text = msg.text.unwrap_or_else(|| "Model switched".to_string());
                self.push_system(&format!("\u{26A0} {text}"));
            }
            "tool_progress" => {
                if let Some(data) = &msg.data {
                    let line = data
                        .get("line")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !line.is_empty() {
                        self.status_tool = Some(line.to_string());
                        // Append to the last InProgress step's output_lines
                        if let Some(group) = &mut self.active_tool_group {
                            if let Some(step) = group.steps.iter_mut().rev().find(|s| s.status == StepStatus::InProgress) {
                                if step.output_lines.len() < 500 {
                                    step.output_lines.push(line.to_string());
                                }
                            }
                        }
                    }
                }
            }
            "content_block" => {
                self.handle_sse_content_block(msg);
            }
            "turn_complete" => {
                self.handle_sse_turn_complete(msg);
            }
            "connection" => {
                match msg.phase.as_deref() {
                    Some("connected") => {
                        self.connection_status = ConnectionStatus::Connected;
                        self.last_seq = 0;
                        self.push_system("Reconnected to server");
                        self.trigger_resync();
                    }
                    Some("disconnected") => {
                        self.connection_status = ConnectionStatus::Disconnected;
                        self.push_system("Disconnected from server — reconnecting…");
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn handle_sse_activity(&mut self, msg: UiSseMessage) {
        let status = msg
            .data
            .as_ref()
            .and_then(|d| d.get("status"))
            .and_then(|v| v.as_str())
            .unwrap_or("working");
        let phase = msg.phase.as_deref().unwrap_or("");
        let text = msg.text.unwrap_or_default();
        let status_id = msg.id.clone();
        let agent = msg
            .agent_id
            .clone()
            .unwrap_or_else(|| self.status_agent.clone());

        // Route subagent activity to its entry instead of the main tool group
        let parent_id = msg
            .data
            .as_ref()
            .and_then(|d| d.get("parent_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let is_subagent = self.subagent_parent_map.contains_key(&agent)
            || parent_id.is_some();

        if is_subagent {
            if let Some(pid) = &parent_id {
                self.subagent_parent_map.entry(agent.clone()).or_insert_with(|| pid.clone());
            }
            if let Some(entry) = self.active_subagents.get_mut(&agent) {
                if status == "calling_tool" {
                    if phase == "doing" {
                        entry.tool_count += 1;
                        let (tool_name, args_summary) = parse_activity_text(&text);
                        entry.tool_steps.push(SubagentToolStep {
                            tool_name,
                            args_summary,
                            status: StepStatus::InProgress,
                        });
                    } else if phase == "done" {
                        if let Some(last) = entry.tool_steps.last_mut() {
                            let is_failed = text.to_lowercase().contains("failed");
                            last.status = if is_failed {
                                StepStatus::Failed
                            } else {
                                StepStatus::Done
                            };
                        }
                    }
                }
                // Update current_activity for non-tool statuses (thinking, model_loading)
                if status == "idle" {
                    entry.current_activity = None;
                } else if status != "calling_tool" && !text.is_empty() {
                    entry.current_activity = Some(text.clone());
                }
            }
            return;
        }

        // Update status bar
        self.status_state = status.to_string();
        self.status_tool = if text.is_empty()
            || text.eq_ignore_ascii_case(status)
        {
            None
        } else {
            Some(text.clone())
        };
        if let Some(aid) = msg.agent_id {
            self.status_agent = aid;
        }

        // Track run start time on first activity for an agent
        if !self.run_start_ts.contains_key(&agent) {
            self.run_start_ts.insert(agent.clone(), Instant::now());
        }

        if status == "calling_tool" {
            if phase == "doing" {
                let (tool_name, args_summary) = parse_activity_text(&text);

                // Dedup: if the active group already has a step with the
                // same status_id, UPDATE it in-place (the server sends
                // "Reading file: X" then "Read file: X" with the same id).
                if let Some(group) = &mut self.active_tool_group {
                    if group.agent_id == agent {
                        if let Some(existing) = group
                            .steps
                            .iter_mut()
                            .find(|s| s.status_id == status_id)
                        {
                            existing.tool_name = tool_name;
                            existing.args_summary = args_summary;
                            return;
                        }
                        // New status_id within the same agent group → new step
                        group.steps.push(ToolStep {
                            status_id,
                            tool_name,
                            args_summary,
                            status: StepStatus::InProgress,
                            output_lines: Vec::new(),
                            result_detail: None,
                        });
                        return;
                    }
                }

                // Different agent or no active group → start fresh
                self.finalize_tool_group();
                self.active_tool_group = Some(ActiveToolGroup {
                    agent_id: agent,
                    steps: vec![ToolStep {
                        status_id,
                        tool_name,
                        args_summary,
                        status: StepStatus::InProgress,
                        output_lines: Vec::new(),
                        result_detail: None,
                    }],
                });
            } else if phase == "done" {
                // Mark the matching step as Done (by status_id, or
                // fall back to the last in-progress step).
                // Also update tool_name/args_summary with done-phase text
                // (e.g. "Reading file: X" → "Read file: X").
                if let Some(group) = &mut self.active_tool_group {
                    let is_failed = text.to_lowercase().contains("failed");
                    let new_status = if is_failed {
                        StepStatus::Failed
                    } else {
                        StepStatus::Done
                    };
                    // Find by status_id first, then fall back to last InProgress
                    let idx = group
                        .steps
                        .iter()
                        .position(|s| s.status_id == status_id)
                        .or_else(|| {
                            group
                                .steps
                                .iter()
                                .rposition(|s| s.status == StepStatus::InProgress)
                        });
                    if let Some(idx) = idx {
                        group.steps[idx].status = new_status;
                        // Update display text from done-phase activity
                        if !text.is_empty() {
                            let (tool_name, args_summary) =
                                parse_activity_text(&text);
                            group.steps[idx].tool_name = tool_name;
                            group.steps[idx].args_summary = args_summary;
                        }
                    }
                }
            }
        } else if status == "idle" {
            // Only finalize on idle (end of run), not on "thinking"
            // which happens between tool calls within the same turn.
            self.finalize_tool_group();
        }
    }

    fn handle_sse_run(&mut self, msg: UiSseMessage) {
        // Trigger a full state resync on sync, resync, or outcome phases
        match msg.phase.as_deref() {
            Some("sync") | Some("resync") | Some("outcome") => {
                self.trigger_resync();
            }
            _ => {}
        }
        if msg.phase.as_deref() == Some("context_usage") {
            if let Some(data) = &msg.data {
                let agent_key = data
                    .get("agent_id")
                    .and_then(|v| v.as_str())
                    .or(msg.agent_id.as_deref())
                    .unwrap_or("")
                    .to_string();
                // Route subagent context to its entry
                if self.subagent_parent_map.contains_key(&agent_key) {
                    if let Some(tokens) = data.get("estimated_tokens").and_then(|v| v.as_u64()) {
                        if let Some(entry) = self.active_subagents.get_mut(&agent_key) {
                            entry.estimated_tokens = Some(tokens as usize);
                        }
                    }
                } else {
                    if let Some(tokens) = data.get("estimated_tokens").and_then(|v| v.as_u64())
                    {
                        self.last_context_tokens
                            .insert(agent_key.clone(), tokens as usize);
                    }
                    if let Some(limit) = data.get("token_limit").and_then(|v| v.as_u64()) {
                        self.last_token_limit
                            .insert(agent_key, limit as usize);
                    }
                }
            }
        }
        if msg.phase.as_deref() == Some("subagent_spawned") {
            if let Some(data) = &msg.data {
                let subagent_id = data
                    .get("subagent_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let parent_id = data
                    .get("parent_id")
                    .and_then(|v| v.as_str())
                    .or(msg.agent_id.as_deref())
                    .unwrap_or("")
                    .to_string();
                let task = data
                    .get("task")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                self.subagent_parent_map.insert(subagent_id.clone(), parent_id);
                let agent_name = subagent_id.clone();
                self.active_subagents.insert(
                    subagent_id.clone(),
                    SubagentEntry {
                        subagent_id,
                        agent_name,
                        task,
                        status: SubagentStatus::Running,
                        tool_count: 0,
                        estimated_tokens: None,
                        current_activity: None,
                        tool_steps: Vec::new(),
                    },
                );
            }
        }
        if msg.phase.as_deref() == Some("subagent_result") {
            if let Some(data) = &msg.data {
                let subagent_id = data
                    .get("subagent_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if let Some(entry) = self.active_subagents.get_mut(&subagent_id) {
                    entry.status = SubagentStatus::Done;
                    entry.current_activity = None;
                }
                self.subagent_parent_map.remove(&subagent_id);
                // Check if all subagents are done
                let all_done = self
                    .active_subagents
                    .values()
                    .all(|e| e.status == SubagentStatus::Done);
                if all_done && !self.active_subagents.is_empty() {
                    self.finalize_subagent_group();
                }
            }
        }
        if msg.phase.as_deref() == Some("plan_update") {
            self.discard_streaming();
            if let Some(data) = &msg.data {
                if let Some(plan) = data.get("plan") {
                    let summary = plan
                        .get("summary")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Plan")
                        .to_string();
                    let status = plan
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("planned")
                        .to_string();

                    // Replace the LAST PlanBlock (regardless of summary)
                    // since the agent may update its plan title mid-run.
                    let replaced = self.blocks.iter_mut().rev().any(|block| {
                        if let DisplayBlock::PlanBlock {
                            summary: existing_summary,
                            status: existing_status,
                            ..
                        } = block
                        {
                            *existing_summary = summary.clone();
                            *existing_status = status.clone();
                            return true;
                        }
                        false
                    });
                    if replaced {
                        // Clear prompt if the updated plan is no longer pending approval
                        if status != "planned" {
                            self.prompt = None;
                        }
                    } else {
                        self.blocks.push(DisplayBlock::PlanBlock {
                            summary,
                            status: status.clone(),
                        });
                        if status == "planned" {
                            let ctx_pct = self.context_usage_pct();
                            let mut options = Vec::new();
                            if ctx_pct >= 40 {
                                options.push(format!(
                                    "Start (new session, {}% context used)",
                                    ctx_pct
                                ));
                            }
                            options.push("Start (continue session)".to_string());
                            options.push("Reject plan".to_string());
                            options.push("Give feedback".to_string());
                            self.prompt = Some(InteractivePrompt {
                                options,
                                selected: 0,
                            });
                        } else {
                            // Clear prompt when plan is no longer pending approval
                            self.prompt = None;
                        }
                    }
                }
            }
        }
        if msg.phase.as_deref() == Some("outcome") {
            self.discard_streaming();
            self.finalize_tool_group();
            self.finalize_subagent_group();
            self.status_state = "idle".to_string();
            self.status_tool = None;
            self.prompt = None;
        }
        if self.session_id.is_none() {
            if let Some(sid) = msg.session_id {
                self.session_id = Some(sid);
            }
        }
    }

    fn handle_sse_content_block(&mut self, msg: UiSseMessage) {
        let phase = msg.phase.as_deref().unwrap_or("");
        let agent = msg.agent_id.clone().unwrap_or_else(|| self.status_agent.clone());
        let data = msg.data.as_ref();
        let parent_id = data
            .and_then(|d| d.get("parent_id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let is_subagent = self.subagent_parent_map.contains_key(&agent)
            || parent_id.is_some();

        // Route subagent content blocks to subagent tracking
        if is_subagent {
            if let Some(pid) = &parent_id {
                self.subagent_parent_map.entry(agent.clone()).or_insert_with(|| pid.clone());
            }
            if let Some(entry) = self.active_subagents.get_mut(&agent) {
                if phase == "start" {
                    let block_type = data.and_then(|d| d.get("block_type")).and_then(|v| v.as_str()).unwrap_or("text");
                    if block_type == "tool_use" {
                        entry.tool_count += 1;
                        let tool_name = data.and_then(|d| d.get("tool")).and_then(|v| v.as_str()).unwrap_or("Tool").to_string();
                        let args_str = data.and_then(|d| d.get("args")).and_then(|v| v.as_str()).unwrap_or("");
                        let args_summary = parse_content_block_args(&tool_name, args_str);
                        entry.tool_steps.push(SubagentToolStep {
                            tool_name,
                            args_summary,
                            status: StepStatus::InProgress,
                        });
                    }
                } else if phase == "update" {
                    let is_error = data.and_then(|d| d.get("is_error")).and_then(|v| v.as_bool()).unwrap_or(false);
                    let status = data.and_then(|d| d.get("status")).and_then(|v| v.as_str()).unwrap_or("");
                    if status == "done" || status == "failed" {
                        if let Some(last) = entry.tool_steps.last_mut() {
                            last.status = if is_error || status == "failed" { StepStatus::Failed } else { StepStatus::Done };
                        }
                    }
                }
            }
            return;
        }

        if phase == "start" {
            let block_type = data.and_then(|d| d.get("block_type")).and_then(|v| v.as_str()).unwrap_or("text");
            if block_type == "tool_use" {
                let tool_name = data.and_then(|d| d.get("tool")).and_then(|v| v.as_str()).unwrap_or("Tool").to_string();
                let args_str = data.and_then(|d| d.get("args")).and_then(|v| v.as_str()).unwrap_or("");
                let block_id = data.and_then(|d| d.get("block_id")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                let args_summary = parse_content_block_args(&tool_name, args_str);

                self.status_state = "calling_tool".to_string();
                self.status_tool = Some(format!("{}: {}", tool_name, args_summary));

                if !self.run_start_ts.contains_key(&agent) {
                    self.run_start_ts.insert(agent.clone(), Instant::now());
                }

                if let Some(group) = &mut self.active_tool_group {
                    if group.agent_id == agent {
                        group.steps.push(ToolStep {
                            status_id: block_id,
                            tool_name,
                            args_summary,
                            status: StepStatus::InProgress,
                            output_lines: Vec::new(),
                            result_detail: None,
                        });
                        return;
                    }
                }
                self.finalize_tool_group();
                self.active_tool_group = Some(ActiveToolGroup {
                    agent_id: agent,
                    steps: vec![ToolStep {
                        status_id: block_id,
                        tool_name,
                        args_summary,
                        status: StepStatus::InProgress,
                        output_lines: Vec::new(),
                        result_detail: None,
                    }],
                });
            }
        } else if phase == "update" {
            let block_id = data.and_then(|d| d.get("block_id")).and_then(|v| v.as_str()).unwrap_or("");
            let status = data.and_then(|d| d.get("status")).and_then(|v| v.as_str()).unwrap_or("");
            let is_error = data.and_then(|d| d.get("is_error")).and_then(|v| v.as_bool()).unwrap_or(false);

            if status == "done" || status == "failed" {
                let new_status = if is_error || status == "failed" { StepStatus::Failed } else { StepStatus::Done };

                // Extract result detail from data
                let result_detail = Self::extract_result_detail(data, &msg.text);

                if let Some(group) = &mut self.active_tool_group {
                    let idx = group.steps.iter().position(|s| s.status_id == block_id)
                        .or_else(|| group.steps.iter().rposition(|s| s.status == StepStatus::InProgress));
                    if let Some(idx) = idx {
                        group.steps[idx].status = new_status;
                        if result_detail.is_some() {
                            group.steps[idx].result_detail = result_detail;
                        }
                        // For Bash: populate output_lines from bash_output if not already populated
                        if group.steps[idx].output_lines.is_empty() {
                            if let Some(bash_output) = data.and_then(|d| d.get("bash_output")).and_then(|v| v.as_str()) {
                                for line in bash_output.lines().take(500) {
                                    group.steps[idx].output_lines.push(line.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Extract a human-readable result detail from content_block update data.
    fn extract_result_detail(
        data: Option<&serde_json::Value>,
        text: &Option<String>,
    ) -> Option<String> {
        let d = data?;
        let tool = d.get("tool").and_then(|v| v.as_str()).unwrap_or("");
        match tool {
            "Write" => {
                if let Some(n) = d.get("lines_written").and_then(|v| v.as_u64()) {
                    return Some(format!("wrote {} lines", n));
                }
                // Fallback: count from summary text
                text.as_deref().map(|t| t.to_string())
            }
            "Edit" => {
                let old_lines = d
                    .get("old_string")
                    .and_then(|v| v.as_str())
                    .map(|s| s.lines().count())
                    .unwrap_or(0);
                let new_lines = d
                    .get("new_string")
                    .and_then(|v| v.as_str())
                    .map(|s| s.lines().count())
                    .unwrap_or(0);
                if old_lines > 0 || new_lines > 0 {
                    return Some(format!("+{} -{} lines", new_lines, old_lines));
                }
                text.as_deref().map(|t| t.to_string())
            }
            "Bash" => {
                if let Some(output) = d.get("bash_output").and_then(|v| v.as_str()) {
                    let line_count = output.lines().count();
                    if line_count > 0 {
                        return Some(format!("{} lines of output", line_count));
                    }
                }
                text.as_deref().map(|t| t.to_string())
            }
            "Read" => {
                // Just use summary text (usually file path)
                text.as_deref().map(|t| t.to_string())
            }
            _ => text.as_deref().map(|t| t.to_string()),
        }
    }

    fn handle_sse_turn_complete(&mut self, msg: UiSseMessage) {
        let agent = msg.agent_id.unwrap_or_default();

        // Skip subagent turn completions
        if self.subagent_parent_map.contains_key(&agent) {
            return;
        }

        let data = msg.data.as_ref();
        let duration_ms = data
            .and_then(|d| d.get("duration_ms"))
            .and_then(|v| v.as_u64());
        let context_tokens = data
            .and_then(|d| d.get("context_tokens"))
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);

        // Fall back to run_start_ts elapsed time
        let elapsed_secs = duration_ms
            .map(|ms| ms / 1000)
            .or_else(|| {
                self.run_start_ts
                    .get(&agent)
                    .map(|t| t.elapsed().as_secs())
            });
        // Fall back to last_context_tokens
        let tokens = context_tokens
            .or_else(|| self.last_context_tokens.get(&agent).copied());

        // Count tool steps from active group before finalizing
        let active_tool_count = self
            .active_tool_group
            .as_ref()
            .map(|g| g.steps.len())
            .unwrap_or(0);

        self.finalize_tool_group();

        // Clean up run tracking
        self.run_start_ts.remove(&agent);

        // Push turn summary footer
        if active_tool_count > 0 || tokens.is_some() || elapsed_secs.is_some() {
            self.blocks.push(DisplayBlock::TurnSummary {
                tool_count: active_tool_count,
                estimated_tokens: tokens,
                duration_secs: elapsed_secs,
            });
        }

        self.status_state = "idle".to_string();
        self.status_tool = None;
    }
}

use super::super::display::*;
use super::{App, AutocompleteItem};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        // When an interactive prompt is active, handle its keys first.
        if self.prompt.is_some() {
            match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(true);
                }
                // Arrow keys navigate the selector
                KeyCode::Up | KeyCode::BackTab => {
                    if let Some(prompt) = &mut self.prompt {
                        prompt.selected = prompt
                            .selected
                            .checked_sub(1)
                            .unwrap_or(prompt.options.len() - 1);
                    }
                }
                KeyCode::Down | KeyCode::Tab => {
                    if let Some(prompt) = &mut self.prompt {
                        prompt.selected = (prompt.selected + 1) % prompt.options.len();
                    }
                }
                // Enter: if input has text, send as free-form; otherwise confirm selection
                KeyCode::Enter => {
                    if !self.input.is_empty() {
                        // Free-form input overrides the selector
                        let custom_text = self.input.trim().to_string();
                        self.input.clear();
                        self.prompt = None;
                        if !custom_text.is_empty() {
                            self.handle_prompt_choice_custom(&custom_text)?;
                        }
                    } else if let Some(prompt) = &self.prompt {
                        let choice = prompt.options[prompt.selected].clone();
                        if choice == "Other..." {
                            // Focus the input box — just dismiss "Other..." label
                            // User can type in the input box directly
                        } else {
                            self.prompt = None;
                            self.handle_prompt_choice(&choice)?;
                        }
                    }
                }
                // Typing goes to the input box (selector stays visible)
                KeyCode::Char(ch) => {
                    self.scroll_offset = 0;
                    self.input.push(ch);
                }
                KeyCode::Backspace => {
                    self.scroll_offset = 0;
                    self.input.pop();
                }
                KeyCode::Esc => {
                    if !self.input.is_empty() {
                        self.input.clear();
                    } else {
                        self.pending_model_select = false;
                        self.prompt = None;
                    }
                }
                _ => {}
            }
            return Ok(false);
        }

        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(true);
            }
            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.paste_clipboard_image();
            }
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.copy_last_agent_message();
            }
            KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.verbose_mode = !self.verbose_mode;
                for block in &mut self.blocks {
                    match block {
                        DisplayBlock::ToolGroup { collapsed, .. }
                        | DisplayBlock::SubagentGroup { collapsed, .. } => {
                            *collapsed = !self.verbose_mode;
                        }
                        _ => {}
                    }
                }
            }
            // Tab: apply autocomplete selection
            KeyCode::Tab if !self.autocomplete.is_empty() => {
                self.apply_autocomplete();
            }
            // Up/Down: navigate autocomplete when visible, otherwise scroll
            KeyCode::Up => {
                if !self.autocomplete.is_empty() {
                    if self.autocomplete_selected == 0 {
                        self.autocomplete_selected = self.autocomplete.len() - 1;
                    } else {
                        self.autocomplete_selected -= 1;
                    }
                } else {
                    self.scroll_offset = self.scroll_offset.saturating_add(1);
                }
            }
            KeyCode::Down => {
                if !self.autocomplete.is_empty() {
                    self.autocomplete_selected =
                        (self.autocomplete_selected + 1) % self.autocomplete.len();
                } else {
                    self.scroll_offset = self.scroll_offset.saturating_sub(1);
                }
            }
            KeyCode::PageUp => {
                self.scroll_offset = self.scroll_offset.saturating_add(20);
            }
            KeyCode::PageDown => {
                self.scroll_offset = self.scroll_offset.saturating_sub(20);
            }
            KeyCode::Char(ch) => {
                self.scroll_offset = 0;
                self.input.push(ch);
                self.update_autocomplete();
            }
            KeyCode::Backspace => {
                self.scroll_offset = 0;
                self.input.pop();
                self.update_autocomplete();
            }
            KeyCode::Enter => {
                if !self.autocomplete.is_empty() {
                    // If input already matches the selected autocomplete label exactly,
                    // send it as a command instead of appending a trailing space.
                    let trimmed = self.input.trim();
                    let exact_match = self.autocomplete.get(self.autocomplete_selected)
                        .map(|item| {
                            let cmd = item.label.split_whitespace().next().unwrap_or(&item.label);
                            trimmed == cmd && !item.label.contains(' ')
                        })
                        .unwrap_or(false);
                    if exact_match {
                        let line = trimmed.to_string();
                        self.input.clear();
                        self.autocomplete.clear();
                        self.scroll_offset = 0;
                        if self.handle_command(line)? {
                            return Ok(true);
                        }
                    } else {
                        // Apply autocomplete selection on Enter (don't send)
                        self.apply_autocomplete();
                    }
                } else {
                    let line = self.input.trim().to_string();
                    self.input.clear();
                    self.autocomplete.clear();
                    if !line.is_empty() {
                        self.scroll_offset = 0;
                        if self.handle_command(line)? {
                            return Ok(true);
                        }
                    }
                }
            }
            KeyCode::Esc => {
                if !self.autocomplete.is_empty() {
                    self.autocomplete.clear();
                } else if !self.input.is_empty() {
                    self.input.clear();
                } else if self.status_state != "idle" {
                    // Cancel running agent
                    self.push_system("Cancelling...");
                    let client = self.client.clone();
                    let project_root = self.project_root.clone();
                    let session_id = self.session_id.clone();
                    tokio::spawn(async move {
                        // Fetch running run_id from API, then cancel
                        match client.fetch_agent_runs(&project_root, session_id.as_deref()).await {
                            Ok(data) => {
                                if let Some(runs) = data.as_array() {
                                    for run in runs {
                                        let status = run.get("status").and_then(|v| v.as_str()).unwrap_or("");
                                        let parent = run.get("parent_run_id").and_then(|v| v.as_str());
                                        if status == "running" && parent.is_none() {
                                            if let Some(run_id) = run.get("run_id").and_then(|v| v.as_str()) {
                                                let _ = client.cancel_run(run_id).await;
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Failed to fetch runs for cancel: {}", e);
                            }
                        }
                    });
                }
            }
            _ => {}
        }
        Ok(false)
    }

    pub(super) fn handle_prompt_choice_custom(&mut self, custom_text: &str) -> Result<bool> {
        // Handle model selector with typed model id.
        if self.pending_model_select {
            self.pending_model_select = false;
            let model_id = custom_text.trim();
            let valid = self.cached_models.is_empty()
                || self.cached_models.iter().any(|(id, _)| id == model_id);
            if !valid {
                self.push_system(&format!("Unknown model: {model_id}"));
                self.push_system("Use /model to see available models.");
            } else {
                self.push_system(&format!("Switching default model to: {model_id}"));
                let client = self.client.clone();
                let mid = model_id.to_string();
                tokio::spawn(async move {
                    if let Err(e) = client.set_default_model(&mid).await {
                        tracing::warn!("Failed to set default model: {}", e);
                    }
                });
            }
            return Ok(false);
        }

        if let Some(question_id) = self.pending_ask_user_id.take() {
            self.push_system(&format!("Other: {}", custom_text));
            let client = self.client.clone();
            let text = custom_text.to_string();
            tokio::spawn(async move {
                if let Err(e) = client.respond_ask_user_custom(&question_id, &text).await {
                    tracing::warn!("AskUser custom response failed: {}", e);
                }
            });
        }
        Ok(false)
    }

    pub(super) fn handle_prompt_choice(&mut self, choice: &str) -> Result<bool> {
        // Handle model selector prompt.
        if self.pending_model_select {
            self.pending_model_select = false;
            // Options are "id[✓]\tdesc" — extract the id part (strip checkmark)
            let name_part = choice.split('\t').next().unwrap_or(choice);
            let model_id = name_part.trim_end_matches(" ✓").trim();
            self.current_default_model = Some(model_id.to_string());
            self.push_system(&format!("Switched default model to: {model_id}"));
            let client = self.client.clone();
            let mid = model_id.to_string();
            tokio::spawn(async move {
                if let Err(e) = client.set_default_model(&mid).await {
                    tracing::warn!("Failed to set default model: {}", e);
                }
            });
            return Ok(false);
        }

        // Handle AskUser/permission prompts.
        if let Some(question_id) = self.pending_ask_user_id.take() {
            self.push_system(&format!("Selected: {}", choice));
            let client = self.client.clone();
            let selected = choice.to_string();
            tokio::spawn(async move {
                if let Err(e) = client.respond_ask_user(&question_id, &selected).await {
                    tracing::warn!("AskUser response failed: {}", e);
                }
            });
            return Ok(false);
        }

        if choice.starts_with("Start (new session") {
            self.push_system("Clearing context and starting plan execution...");
            let client = self.client.clone();
            let project_root = self.project_root.clone();
            let agent_id = self.agent_id.clone();
            let session_id = self.session_id.clone();
            tokio::spawn(async move {
                if let Err(e) = client
                    .approve_plan(&project_root, &agent_id, session_id.as_deref(), true)
                    .await
                {
                    tracing::warn!("Plan approve failed: {}", e);
                }
            });
        } else if choice == "Start (continue session)" {
            self.push_system("Starting plan execution...");
            let client = self.client.clone();
            let project_root = self.project_root.clone();
            let agent_id = self.agent_id.clone();
            let session_id = self.session_id.clone();
            tokio::spawn(async move {
                if let Err(e) = client
                    .approve_plan(&project_root, &agent_id, session_id.as_deref(), false)
                    .await
                {
                    tracing::warn!("Plan approve failed: {}", e);
                }
            });
        } else if choice == "Reject plan" {
            self.push_system("Rejecting plan...");
            let client = self.client.clone();
            let project_root = self.project_root.clone();
            let agent_id = self.agent_id.clone();
            let session_id = self.session_id.clone();
            tokio::spawn(async move {
                if let Err(e) = client
                    .reject_plan(&project_root, &agent_id, session_id.as_deref())
                    .await
                {
                    tracing::warn!("Plan reject failed: {}", e);
                }
            });
        } else if choice == "Give feedback" {
            self.push_system("Type your feedback and press Enter:");
            self.input = "/plan feedback ".to_string();
        }
        Ok(false)
    }

    pub(super) fn handle_command(&mut self, line: String) -> Result<bool> {
        if line == "/quit" || line == "/exit" {
            return Ok(true);
        }

        if line == "/help" {
            self.push_system("Commands:");
            self.push_system("  /agent <name>     switch default agent");
            self.push_system("  /model            select default model");
            self.push_system("  /clear            clear chat context");
            self.push_system("  /status           show project status");
            self.push_system("  @agent message    send to specific agent");
            self.push_system("  /plan <task>      ask agent to create a plan (read-only)");
            self.push_system("  /plan approve     approve and execute the pending plan");
            self.push_system("  /plan reject      reject the pending plan");
            self.push_system("  /copy             copy last agent message to clipboard");
            self.push_system("  /image <path>     attach an image file");
            self.push_system("  /image clear      remove all pending images");
            self.push_system("  /paste            paste image from clipboard");
            self.push_system("  /quit, /exit      exit");
            self.push_system("  <text>            send message to current agent");
            self.push_system("  Ctrl+Y            copy last agent message to clipboard");
            self.push_system("  Ctrl+V            paste image from clipboard");
            self.push_system("  Ctrl+O            toggle verbose/compact tool display");
            self.push_system("  Esc               cancel running agent / clear input");
            self.push_system("  ↑/↓, scroll       scroll output");
            self.push_system("  PgUp/PgDn         scroll output (fast)");
            return Ok(false);
        }

        // Plan commands
        if line == "/plan" {
            self.push_system("Usage: /plan <task>  — create a plan");
            self.push_system("       /plan approve — approve and execute");
            self.push_system("       /plan reject  — reject the plan");
            return Ok(false);
        }
        if line == "/plan approve" {
            self.push_system("Approving plan...");
            let client = self.client.clone();
            let project_root = self.project_root.clone();
            let agent_id = self.agent_id.clone();
            let session_id = self.session_id.clone();
            tokio::spawn(async move {
                if let Err(e) = client
                    .approve_plan(&project_root, &agent_id, session_id.as_deref(), false)
                    .await
                {
                    tracing::warn!("Plan approve failed: {}", e);
                }
            });
            return Ok(false);
        }
        if line == "/plan reject" {
            self.push_system("Rejecting plan...");
            let client = self.client.clone();
            let project_root = self.project_root.clone();
            let agent_id = self.agent_id.clone();
            let session_id = self.session_id.clone();
            tokio::spawn(async move {
                if let Err(e) = client
                    .reject_plan(&project_root, &agent_id, session_id.as_deref())
                    .await
                {
                    tracing::warn!("Plan reject failed: {}", e);
                }
            });
            return Ok(false);
        }
        if line.starts_with("/plan ") {
            self.push_user(&line);
            self.pending_user_messages.push_back(line.clone());
            self.status_state = "sending".to_string();
            let client = self.client.clone();
            let project_root = self.project_root.clone();
            let agent_id = self.agent_id.clone();
            let session_id = self.session_id.clone();
            let slot = self.session_id_slot.clone();
            let msg = line.clone();
            tokio::spawn(async move {
                if let Ok(Some(sid)) = client
                    .send_chat(&project_root, &agent_id, &msg, session_id.as_deref(), None)
                    .await
                {
                    let mut guard = slot.lock().unwrap();
                    if guard.is_none() {
                        *guard = Some(sid);
                    }
                }
            });
            return Ok(false);
        }

        // Status command
        if line == "/status" {
            self.push_system("Loading status...");
            let client = self.client.clone();
            let project_root = self.project_root.clone();
            let slot = self.status_lines_slot.clone();
            tokio::spawn(async move {
                match client.fetch_status(&project_root).await {
                    Ok(data) => {
                        let mut lines = Vec::new();
                        lines.push(String::new());
                        lines.push("  ─── Project Status ───".to_string());
                        lines.push(String::new());

                        // Default model
                        let default_model = data.get("default_model")
                            .and_then(|v| v.as_str())
                            .unwrap_or("(none)");
                        lines.push(format!("  Default model:  {default_model}"));
                        lines.push(String::new());

                        // Available models
                        if let Some(models) = data.get("models").and_then(|v| v.as_array()) {
                            lines.push("  Models:".to_string());
                            for m in models {
                                let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                                let provider = m.get("provider").and_then(|v| v.as_str()).unwrap_or("?");
                                let model = m.get("model").and_then(|v| v.as_str()).unwrap_or("?");
                                let is_default = id == default_model;
                                let mark = if is_default { " ✓" } else { "" };
                                lines.push(format!("    {id}{mark}  ({provider}: {model})"));
                            }
                            lines.push(String::new());
                        }

                        // Sessions & runs
                        let sessions = data.get("sessions").and_then(|v| v.as_u64()).unwrap_or(0);
                        let total = data.get("total_runs").and_then(|v| v.as_u64()).unwrap_or(0);
                        let completed = data.get("completed_runs").and_then(|v| v.as_u64()).unwrap_or(0);
                        let failed = data.get("failed_runs").and_then(|v| v.as_u64()).unwrap_or(0);
                        let cancelled = data.get("cancelled_runs").and_then(|v| v.as_u64()).unwrap_or(0);
                        let active_days = data.get("active_days").and_then(|v| v.as_u64()).unwrap_or(0);

                        lines.push(format!("  Sessions:       {sessions:<12}  Runs:         {total}"));
                        lines.push(format!("  Completed:      {completed:<12}  Failed:       {failed}"));
                        lines.push(format!("  Cancelled:      {cancelled:<12}  Active days:  {active_days}"));
                        lines.push(String::new());

                        // Session tokens
                        let prompt_tok = data.get("session_prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                        let completion_tok = data.get("session_completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                        if prompt_tok > 0 || completion_tok > 0 {
                            let fmt = |n: u64| -> String {
                                if n >= 1_000_000 { format!("{:.1}M", n as f64 / 1_000_000.0) }
                                else if n >= 1_000 { format!("{:.1}K", n as f64 / 1_000.0) }
                                else { format!("{n}") }
                            };
                            let total = prompt_tok + completion_tok;
                            lines.push(format!("  Session tokens: ↑ {}  ↓ {}  (total: {})", fmt(prompt_tok), fmt(completion_tok), fmt(total)));
                            lines.push(String::new());
                        }

                        // Model usage
                        if let Some(usage) = data.get("model_usage").and_then(|v| v.as_array()) {
                            if !usage.is_empty() {
                                lines.push("  Model usage:".to_string());
                                for entry in usage {
                                    if let Some(arr) = entry.as_array() {
                                        let name = arr.first().and_then(|v| v.as_str()).unwrap_or("?");
                                        let count = arr.get(1).and_then(|v| v.as_u64()).unwrap_or(0);
                                        lines.push(format!("    {name:<30} {count} runs"));
                                    }
                                }
                                lines.push(String::new());
                            }
                        }

                        *slot.lock().unwrap() = Some(lines);
                    }
                    Err(e) => {
                        *slot.lock().unwrap() = Some(vec![format!("  Error fetching status: {e}")]);
                    }
                }
            });
            return Ok(false);
        }

        // Copy command
        if line == "/copy" {
            self.copy_last_agent_message();
            return Ok(false);
        }

        // Clear context
        if line == "/clear" {
            self.blocks.clear();
            self.active_tool_group = None;
            self.scroll_offset = 0;
            self.push_system("Context cleared.");
            let client = self.client.clone();
            let project_root = self.project_root.clone();
            let session_id = self.session_id.clone();
            tokio::spawn(async move {
                if let Err(e) = client.clear_chat(&project_root, session_id.as_deref()).await {
                    tracing::warn!("Failed to clear chat: {}", e);
                }
            });
            return Ok(false);
        }

        // Image commands
        if line == "/image" || line.starts_with("/image ") || line == "/paste" {
            self.handle_image_command(&line);
            return Ok(false);
        }

        if let Some(rest) = line.strip_prefix("/agent ") {
            let name = rest.trim().to_string();
            self.agent_id = name.clone();
            self.status_agent = name.clone();
            self.push_system(&format!("Switched to agent: {name}"));
            return Ok(false);
        }

        // Model commands
        if line == "/model" {
            if self.cached_models.is_empty() {
                self.push_system("No models available (loading...)");
            } else {
                let default_id = self.current_default_model.as_deref();
                let mut default_idx = 0;
                let options: Vec<String> = self
                    .cached_models
                    .iter()
                    .enumerate()
                    .map(|(i, (id, desc))| {
                        let is_default = default_id == Some(id.as_str());
                        if is_default {
                            default_idx = i;
                        }
                        let mark = if is_default { " ✓" } else { "" };
                        format!("{id}{mark}\t{desc}")
                    })
                    .collect();
                self.prompt = Some(super::InteractivePrompt {
                    options,
                    selected: default_idx,
                });
                self.pending_model_select = true;
            }
            return Ok(false);
        }
        if let Some(rest) = line.strip_prefix("/model ") {
            let model_id = rest.trim().to_string();
            if model_id.is_empty() {
                self.push_system("Usage: /model <id>");
                return Ok(false);
            }
            // Validate model_id exists in cached models
            let valid = self.cached_models.is_empty()
                || self.cached_models.iter().any(|(id, _)| id == &model_id);
            if !valid {
                self.push_system(&format!("Unknown model: {model_id}"));
                self.push_system("Use /model to see available models.");
                return Ok(false);
            }
            self.push_system(&format!("Switching default model to: {model_id}"));
            let client = self.client.clone();
            let mid = model_id.clone();
            tokio::spawn(async move {
                if let Err(e) = client.set_default_model(&mid).await {
                    tracing::warn!("Failed to set default model: {}", e);
                }
            });
            return Ok(false);
        }

        // @agent_id message — one-shot to a specific agent
        let (target_agent, message) = if line.starts_with('@') {
            if let Some(pos) = line[1..].find(' ') {
                let agent = line[1..1 + pos].to_string();
                let msg = line[2 + pos..].trim().to_string();
                (agent, msg)
            } else {
                (self.agent_id.clone(), line)
            }
        } else {
            (self.agent_id.clone(), line)
        };

        let images = std::mem::take(&mut self.pending_images);
        let image_count = images.len();
        self.push_user_with_images(&message, image_count);
        self.pending_user_messages.push_back(message.clone());
        self.status_state = "sending".to_string();

        let client = self.client.clone();
        let project_root = self.project_root.clone();
        let session_id = self.session_id.clone();
        let slot = self.session_id_slot.clone();

        let send_images = if images.is_empty() { None } else { Some(images) };
        tokio::spawn(async move {
            if let Ok(Some(sid)) = client
                .send_chat(
                    &project_root,
                    &target_agent,
                    &message,
                    session_id.as_deref(),
                    send_images,
                )
                .await
            {
                let mut guard = slot.lock().unwrap();
                if guard.is_none() {
                    *guard = Some(sid);
                }
            }
        });

        Ok(false)
    }

    /// Built-in slash commands for autocomplete.
    const BUILTIN_COMMANDS: &'static [(&'static str, &'static str)] = &[
        ("/help", "Show available commands"),
        ("/agent <name>", "Switch default agent"),
        ("/model <id>", "Switch default model"),
        ("/clear", "Clear chat context"),
        ("/status", "Show project status"),
        ("/plan <task>", "Create a plan"),
        ("/copy", "Copy last response to clipboard"),
        ("/image <path>", "Attach an image file"),
        ("/paste", "Paste image from clipboard"),
        ("/quit", "Exit"),
    ];

    /// Update autocomplete suggestions based on current input.
    fn update_autocomplete(&mut self) {
        let input = self.input.as_str();

        // Model autocomplete: "/model " followed by partial model id
        if let Some(partial) = input.strip_prefix("/model ") {
            let filter = partial.to_lowercase();
            let items: Vec<AutocompleteItem> = self
                .cached_models
                .iter()
                .filter(|(id, _)| filter.is_empty() || id.to_lowercase().contains(&filter))
                .map(|(id, desc)| AutocompleteItem {
                    label: format!("/model {id}"),
                    description: desc.clone(),
                })
                .collect();
            self.autocomplete = items;
            self.autocomplete_selected = 0;
            return;
        }

        // Only trigger on prefix with no spaces (still typing the command/agent name)
        if input.starts_with('/') && !input[1..].contains(' ') {
            let filter = input[1..].to_lowercase();
            let mut items: Vec<AutocompleteItem> = Vec::new();

            // Built-in commands
            for &(cmd, desc) in Self::BUILTIN_COMMANDS {
                let cmd_name = cmd.split_whitespace().next().unwrap_or(cmd);
                if filter.is_empty() || cmd_name[1..].to_lowercase().contains(&filter) {
                    items.push(AutocompleteItem {
                        label: cmd.to_string(),
                        description: desc.to_string(),
                    });
                }
            }

            // Cached skills
            for (name, desc) in &self.cached_skills {
                let prefixed = format!("/{name}");
                if filter.is_empty() || name.to_lowercase().contains(&filter) {
                    items.push(AutocompleteItem {
                        label: prefixed,
                        description: desc.clone(),
                    });
                }
            }

            self.autocomplete = items;
            self.autocomplete_selected = 0;
        } else if input.starts_with('@') && !input[1..].contains(' ') {
            let filter = input[1..].to_lowercase();
            let mut items: Vec<AutocompleteItem> = Vec::new();

            for (name, desc) in &self.cached_agents {
                if filter.is_empty() || name.to_lowercase().contains(&filter) {
                    items.push(AutocompleteItem {
                        label: format!("@{name}"),
                        description: desc.clone(),
                    });
                }
            }

            self.autocomplete = items;
            self.autocomplete_selected = 0;
        } else {
            self.autocomplete.clear();
        }
    }

    /// Apply the currently selected autocomplete item to the input.
    fn apply_autocomplete(&mut self) {
        if let Some(item) = self.autocomplete.get(self.autocomplete_selected) {
            // For commands with parameters like "/agent <name>", use just the command word
            let label = &item.label;
            if label.contains(' ') {
                // e.g. "/agent <name>" → set input to "/agent "
                let cmd_part = label.split_whitespace().next().unwrap_or(label);
                self.input = format!("{cmd_part} ");
            } else {
                self.input = format!("{label} ");
            }
        }
        self.autocomplete.clear();
    }
}

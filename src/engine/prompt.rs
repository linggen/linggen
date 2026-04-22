use super::core_memory;
use super::memory;
use super::types::*;
use crate::engine::tools;
use crate::ollama::ChatMessage;
use std::collections::HashSet;
use std::sync::OnceLock;

fn get_os_version() -> String {
    static OS_VERSION: OnceLock<String> = OnceLock::new();
    OS_VERSION
        .get_or_init(|| {
            #[cfg(unix)]
            {
                std::process::Command::new("uname")
                    .args(["-rs"])
                    .output()
                    .ok()
                    .and_then(|o| {
                        if o.status.success() {
                            String::from_utf8(o.stdout).ok().map(|s| s.trim().to_string())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| "unknown".into())
            }
            #[cfg(not(unix))]
            {
                "unknown".into()
            }
        })
        .clone()
}

fn workspace_listing(ws_root: &std::path::Path) -> String {
    let entries = match std::fs::read_dir(ws_root) {
        Ok(e) => e,
        Err(_) => return String::new(),
    };
    let mut items: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') && !matches!(name.as_str(),
            ".claude" | ".linggen" | ".git" | ".github" | ".vscode" | ".cursorrules"
        ) {
            continue;
        }
        let is_dir = entry.file_type().map_or(false, |ft| ft.is_dir());
        items.push(format!("  {}{}", name, if is_dir { "/" } else { "" }));
        if items.len() >= 50 {
            items.push("  ... (truncated)".to_string());
            break;
        }
    }
    items.sort();
    items.join("\n")
}

impl AgentEngine {
    pub(crate) fn system_prompt(&self) -> String {
        use crate::prompts::keys;

        // Personality is injected first — it's the agent's voice regardless of context.
        let personality = self
            .spec
            .as_ref()
            .and_then(|s| s.personality.as_deref())
            .unwrap_or("");

        // App skills override the agent body — the agent's coding/workflow instructions
        // are irrelevant when the skill runs its own UI (e.g. game-table).
        // The agent's personality traits still carry through.
        let is_app_skill = self
            .active_skill
            .as_ref()
            .is_some_and(|s| s.app.is_some());

        let body = if is_app_skill || self.prompt_profile.consumer_frame {
            // App skills: skill content becomes the primary prompt.
            // Consumer sessions: agent spec body describes owner capabilities
            // (coding, delegation, file editing) that consumers don't have.
            // Skip it — the consumer frame in build_stable_system_content
            // provides appropriate instructions.
            String::new()
        } else {
            self.spec_system_prompt
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    self.prompt_store
                        .render_or_fallback(keys::SYSTEM_FALLBACK_IDENTITY, &[])
                })
        };

        let mut prompt = if personality.is_empty() {
            body
        } else if body.is_empty() {
            personality.trim().to_string()
        } else {
            format!("{}\n\n{}", personality.trim(), body)
        };

        // Don't list available skills for app skill sessions — the model
        // should focus entirely on the active skill.
        if !is_app_skill && !self.available_skills_metadata.is_empty() {
            // Filter by consumer_allowed_skills when in consumer mode.
            let skills: Vec<&(String, String)> = match &self.cfg.consumer_allowed_skills {
                Some(allowed) => self.available_skills_metadata.iter()
                    .filter(|(name, _)| allowed.contains(name))
                    .collect(),
                None => self.available_skills_metadata.iter().collect(),
            };
            if !skills.is_empty() {
                prompt.push_str(&self.prompt_store.render_or_fallback(
                    keys::SYSTEM_SKILLS_HEADER,
                    &[],
                ));
                for (name, description) in skills {
                    prompt.push_str(&self.prompt_store.render_or_fallback(
                        keys::SYSTEM_SKILL_ENTRY,
                        &[("name", name.as_str()), ("description", description.as_str())],
                    ));
                }
            }
        }

        if let Some(skill) = &self.active_skill {
            // Replace $SKILL_DIR so the model sees the actual filesystem path.
            let resolved_content = if let Some(ref dir) = skill.skill_dir {
                skill.content.replace("$SKILL_DIR", &dir.to_string_lossy())
            } else {
                skill.content.clone()
            };
            prompt.push_str(&self.prompt_store.render_or_fallback(
                keys::SYSTEM_ACTIVE_SKILL_FRAME,
                &[
                    ("name", skill.name.as_str()),
                    ("description", skill.description.as_str()),
                    ("content", &resolved_content),
                ],
            ));

            // App-skills receive the built-in PageUpdate tool. Remind the
            // model to call it whenever state the user should see has changed —
            // unless the skill body already documents PageUpdate itself, in
            // which case the generic hint is redundant duplication.
            if skill.app.is_some() && !resolved_content.contains("PageUpdate") {
                prompt.push_str(&self.prompt_store.render_or_fallback(
                    keys::SYSTEM_APP_SKILL_DASHBOARD_HINT,
                    &[],
                ));
            }
        }

        prompt
    }

    /// Build the stable portion of the system prompt (agent spec + project context + memory)
    /// and return (content, hash). This is the cacheable prefix.
    pub(crate) fn build_stable_system_content(&self) -> (String, u64) {
        use crate::prompts::keys;
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut stable = self.system_prompt();

        // --- Environment block (owner only) ---
        if self.prompt_profile.include_environment {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "unknown".into());
            let os_version = get_os_version();
            stable.push_str(&self.prompt_store.render_or_fallback(
                keys::SYSTEM_ENVIRONMENT_BLOCK,
                &[
                    ("platform", std::env::consts::OS),
                    ("os_version", &os_version),
                    ("shell", &shell),
                    ("ws_root", &self.cfg.ws_root.display().to_string()),
                    ("interface_mode", &self.cfg.interface_mode.to_string()),
                ],
            ));
        }

        // --- Project context files (owner only) ---
        if self.prompt_profile.include_project_context {
            let context_filenames = ["AGENTS.md", "CLAUDE.md", ".cursorrules"];
            let mut seen: std::collections::HashSet<std::path::PathBuf> =
                std::collections::HashSet::new();
            let mut sections: Vec<(String, String)> = Vec::new();

            let mut dir: Option<&std::path::Path> = Some(self.cfg.ws_root.as_path());
            while let Some(current) = dir {
                for filename in &context_filenames {
                    let filepath = current.join(filename);
                    if let Ok(canonical) = filepath.canonicalize() {
                        if seen.contains(&canonical) {
                            continue;
                        }
                        if let Ok(content) = std::fs::read_to_string(&filepath) {
                            let content = content.trim().to_string();
                            if !content.is_empty() {
                                let label = if current == self.cfg.ws_root.as_path() {
                                    filename.to_string()
                                } else {
                                    format!("{} (from {})", filename, current.display())
                                };
                                sections.push((label, content));
                                seen.insert(canonical);
                            }
                        }
                    }
                }
                dir = current.parent();
            }
            sections.reverse();
            if !sections.is_empty() {
                stable.push_str(&self.prompt_store.render_or_fallback(
                    keys::SYSTEM_PROJECT_INSTRUCTIONS_HEADER,
                    &[],
                ));
                for (label, content) in &sections {
                    stable.push_str(&self.prompt_store.render_or_fallback(
                        keys::SYSTEM_PROJECT_INSTRUCTIONS_ENTRY,
                        &[("label", label.as_str()), ("content", content.as_str())],
                    ));
                }
                stable.push_str(&self.prompt_store.render_or_fallback(
                    keys::SYSTEM_PROJECT_INSTRUCTIONS_FOOTER,
                    &[],
                ));
            }
        }

        // --- Core memory (owner only) ---
        // Layer 1: ~/.linggen/core/identity.md + style.md — universals
        // inlined into the stable prompt. When files are still unedited
        // scaffolding, emit the bootstrap hint instead so the model knows
        // how to initialize memory. Layer 2 (facts / activity / semantic
        // retrieval) reaches the model through Memory_* tools registered
        // when a `provides: [memory]` skill is active — not through here.
        if self.prompt_profile.include_memory {
            let core_dir_display = crate::paths::memory_dir().display().to_string();
            match core_memory::load_core() {
                Some(c) => stable.push_str(&self.prompt_store.render_or_fallback(
                    keys::CORE_MEMORY_BLOCK,
                    &[
                        ("core_dir", &core_dir_display),
                        ("identity", &c.identity),
                        ("style", &c.style),
                    ],
                )),
                None => stable.push_str(&self.prompt_store.render_or_fallback(
                    keys::CORE_MEMORY_BLOCK_EMPTY,
                    &[("core_dir", &core_dir_display)],
                )),
            }
        }

        // --- Consumer frame (consumer only) ---
        if self.prompt_profile.consumer_frame {
            stable.push_str(&self.prompt_store.render_or_fallback(
                keys::SYSTEM_CONSUMER_FRAME,
                &[],
            ));
        }

        let mut hasher = DefaultHasher::new();
        stable.hash(&mut hasher);
        let hash = hasher.finish();
        (stable, hash)
    }

    /// Build the initial message list and read-paths set for the structured agent loop.
    /// When `native_tools` is true, uses the native tool calling response format
    /// (no JSON action format instructions) instead of the legacy format.
    pub(crate) fn prepare_loop_messages(
        &mut self,
        task: &str,
        native_tools: bool,
    ) -> (Vec<ChatMessage>, Option<HashSet<String>>, HashSet<String>) {
        // Build stable system content with caching.
        let (stable_content, hash) = self.build_stable_system_content();
        let cache_hit = self.cached_system_prompt.as_ref().map_or(false, |c| c.input_hash == hash);
        if !cache_hit {
            self.cached_system_prompt = Some(CachedSystemPrompt {
                input_hash: hash,
                content: stable_content.clone(),
            });
        }
        let mut system = self.cached_system_prompt.as_ref().unwrap().content.clone();

        // Compute allowed tools early — needed for the response format schema.
        let mut allowed_tools = self.allowed_tool_names();
        if self.plan_mode {
            let read_only: HashSet<String> = [
                "Read", "Glob", "Grep", "WebSearch", "WebFetch", "AskUser", "ExitPlanMode", "UpdatePlan", "Task",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect();
            allowed_tools = Some(match allowed_tools {
                Some(existing) => existing.intersection(&read_only).cloned().collect(),
                None => read_only,
            });
        }

        // Apply config-level tool restrictions (mission tiers + consumer room settings).
        // Uses a single helper that computes the cascading intersection.
        if let Some(restrictions) = self.cfg.effective_tool_restrictions() {
            allowed_tools = Some(match allowed_tools {
                Some(existing) => existing.intersection(&restrictions).cloned().collect(),
                None => restrictions,
            });
        }

        // Dynamic content appended after cached stable prefix.

        // Check if tools are available — skip all tool-related prompt sections when empty.
        let has_tools = allowed_tools.as_ref().map_or(true, |s| !s.is_empty());

        // --- Response Format ---
        if has_tools {
            if native_tools {
                // Native tool calling: model gets tool schemas via the API `tools` parameter.
                // Use a lightweight prompt with usage guidelines only (no JSON format
                // instructions). Sections that reference specific tools (AskUser, Plan
                // Mode, UpdatePlan, Task delegation) are appended only when those tools
                // are actually in `allowed_tools`. Advertising a tool the session can't
                // call wastes tokens and invites failed calls.
                let tool_allowed = |name: &str| -> bool {
                    allowed_tools.as_ref().map_or(true, |s| s.contains(name))
                };
                if let Some(base) = self.prompt_store.get(crate::prompts::RESPONSE_FORMAT_NATIVE) {
                    system.push_str("\n\n");
                    system.push_str(base);
                    if tool_allowed("AskUser") {
                        if let Some(b) = self.prompt_store.get(
                            crate::prompts::keys::RESPONSE_FORMAT_NATIVE_ASKUSER_BULLET,
                        ) {
                            system.push_str(b);
                        }
                    }
                    if let Some(c) = self.prompt_store.get(
                        crate::prompts::keys::RESPONSE_FORMAT_NATIVE_CONVERSATIONAL,
                    ) {
                        system.push_str(c);
                    }
                    if tool_allowed("EnterPlanMode") {
                        if let Some(p) = self.prompt_store.get(
                            crate::prompts::keys::RESPONSE_FORMAT_NATIVE_PLAN_MODE,
                        ) {
                            system.push_str(p);
                        }
                    }
                    if tool_allowed("UpdatePlan") {
                        if let Some(u) = self.prompt_store.get(
                            crate::prompts::keys::RESPONSE_FORMAT_NATIVE_UPDATE_PLAN,
                        ) {
                            system.push_str(u);
                        }
                    }
                    if let Some(r) = self.prompt_store.get(
                        crate::prompts::keys::RESPONSE_FORMAT_NATIVE_RULES_BASE,
                    ) {
                        system.push_str(r);
                    }
                    if tool_allowed("Task") {
                        if let Some(t) = self.prompt_store.get(
                            crate::prompts::keys::RESPONSE_FORMAT_NATIVE_RULES_TASK,
                        ) {
                            system.push_str(t);
                        }
                    }
                }
            } else {
                // Legacy mode: inject JSON action format + inline tool schemas.
                let tools_json = self.tools.tool_schema_json(allowed_tools.as_ref());
                if let Some(rendered) = self.prompt_store.render(
                    crate::prompts::RESPONSE_FORMAT,
                    &[("tools", &tools_json)],
                ) {
                    system.push_str("\n\n");
                    system.push_str(&rendered);
                }
            }
        }

        // Plan mode: restrict to read-only tools and instruct the model to produce a plan.
        if has_tools && self.plan_mode {
            if let Some(content) = self.prompt_store.get(crate::prompts::PLAN_MODE) {
                system.push_str("\n\n");
                system.push_str(content);
            }
        }

        // Inject available agents for Task delegation (owner only).
        if has_tools && self.prompt_profile.include_delegation && !self.available_agents_metadata.is_empty() {
            let task_available = allowed_tools
                .as_ref()
                .map_or(true, |s| s.contains("Task"));
            if task_available {
                system.push_str(&self.prompt_store.render_or_fallback(
                    crate::prompts::keys::SYSTEM_DELEGATION_HEADER,
                    &[],
                ));
                for (name, description) in &self.available_agents_metadata {
                    system.push_str(&self.prompt_store.render_or_fallback(
                        crate::prompts::keys::SYSTEM_DELEGATION_ENTRY,
                        &[("name", name.as_str()), ("description", description.as_str())],
                    ));
                }
                system.push('\n');
            }
        }

        // If executing an approved plan, inject the plan into the prompt.
        if let Some(plan) = &self.plan {
            if plan.status == PlanStatus::Approved || plan.status == PlanStatus::Executing {
                if let Some(rendered) = self.prompt_store.render(
                    crate::prompts::PLAN_EXECUTE,
                    &[("plan_text", &plan.plan_text)],
                ) {
                    system.push_str("\n\n");
                    system.push_str(&rendered);
                }
            }
        }

        let mut messages = vec![ChatMessage::new("system", system)];
        self.push_context_record(
            ContextType::System,
            Some("structured_loop_prompt".to_string()),
            None,
            None,
            messages[0].content.clone(),
            serde_json::json!({ "mode": "structured" }),
        );

        // Include chat history so the model has context of the current conversation.
        messages.extend(self.chat_history.clone());

        // Memory self-review nudge — every N user messages, remind the model
        // to check whether the recent exchange produced anything worth saving
        // or updating in memory. Only fires when memory is enabled for the
        // session (owner chats); never in consumer/mission sessions.
        if self.prompt_profile.include_memory
            && memory::should_fire_nudge(&self.chat_history, self.cfg.memory_nudge_interval)
        {
            messages.push(memory::nudge_message());
        }

        for obs in &self.observations {
            messages.push(ChatMessage::new("user", self.observation_for_model(obs)));
        }

        // Provide workspace info + task (last user message).
        // Owner gets full workspace listing; consumer gets task only.
        let task_content = if self.prompt_profile.include_workspace_listing {
            let ws_listing = workspace_listing(&self.cfg.ws_root);
            self.prompt_store.render(
                crate::prompts::TASK_BOOTSTRAP,
                &[
                    ("ws_root", &self.cfg.ws_root.display().to_string()),
                    ("platform", std::env::consts::OS),
                    ("role", &format!("{:?}", self.role)),
                    ("workspace_listing", &ws_listing),
                    ("task", task),
                ],
            ).unwrap_or_else(|| format!(
                "Autonomous agent loop started.\n\nWorkspace root: {}\nPlatform: {}\nCurrent Role: {:?}\n\nWorkspace contents:\n{}\n\nTask: {}",
                self.cfg.ws_root.display(), std::env::consts::OS, self.role, ws_listing, task,
            ))
        } else {
            task.to_string()
        };
        let task_msg = ChatMessage::new("user", task_content);
        // Attach any pending images to the task message, then clear them.
        let images = std::mem::take(&mut self.pending_images);
        if !images.is_empty() {
            tracing::info!("Attaching {} inline image(s) to task message ({} bytes total)",
                images.len(),
                images.iter().map(|i| i.len()).sum::<usize>());
        }
        let task_msg = if images.is_empty() { task_msg } else { task_msg.with_images(images) };
        messages.push(task_msg);
        self.push_context_record(
            ContextType::UserInput,
            Some("structured_bootstrap".to_string()),
            Some("system".to_string()),
            self.agent_id.clone(),
            messages
                .last()
                .map(|m| m.content.clone())
                .unwrap_or_default(),
            serde_json::json!({ "source": "run_agent_loop" }),
        );

        // Pre-populate read_paths from prior context.
        let mut read_paths: HashSet<String> = HashSet::new();
        let base_dir = self.tools.builtins.cwd();
        let mut ingest_read_file_text = |text: &str| {
            if !text.contains("Read:") || text.contains("tool_error:") {
                return;
            }
            if let Some(start) = text.find("Read: ") {
                let path_part = &text[start + 6..];
                let raw_path = path_part.split_whitespace().next().unwrap_or("");
                if raw_path.is_empty() {
                    return;
                }
                let clean_path = raw_path
                    .trim_end_matches(')')
                    .trim_end_matches(',')
                    .trim_end_matches('.')
                    .to_string();
                if clean_path.is_empty() {
                    return;
                }
                read_paths.insert(clean_path.clone());
                if let Ok(abs) = base_dir.join(&clean_path).canonicalize() {
                    if let Ok(rel) = abs.strip_prefix(&base_dir) {
                        read_paths.insert(rel.to_string_lossy().to_string());
                    }
                }
            }
        };
        for obs in &self.observations {
            if obs.name == "Read" {
                ingest_read_file_text(&obs.content);
            }
        }
        for msg in &self.chat_history {
            ingest_read_file_text(&msg.content);
        }

        (messages, allowed_tools, read_paths)
    }

    // -----------------------------------------------------------------------
    // Tool filtering
    // -----------------------------------------------------------------------

    pub(crate) fn allowed_tool_names(&self) -> Option<HashSet<String>> {
        // When a skill is active and declares allowed-tools, those take
        // precedence — the agent can only use the tools the skill permits.
        if let Some(skill) = &self.active_skill {
            if let Some(ref tools_list) = skill.allowed_tools {
                if tools_list.is_empty() {
                    // allowed-tools: [] → no tools at all (not even Skill)
                    return Some(HashSet::new());
                }
                let mut allowed = tools_list
                    .iter()
                    .filter_map(|tool| {
                        if let Some(name) = tools::canonical_tool_name(tool) {
                            return Some(name.to_string());
                        }
                        if self.tools.has_skill_tool(tool) {
                            return Some(tool.to_string());
                        }
                        None
                    })
                    .collect::<HashSet<String>>();
                // The active skill's own custom tools are always allowed.
                for td in &skill.tool_defs {
                    allowed.insert(td.name.clone());
                }
                // Skill tool is always allowed so the model can discover/invoke skills.
                allowed.insert("Skill".to_string());
                // Read/Write/Edit are always allowed when the built-in memory
                // directory exists, so the model can read and update
                // identity.md / style.md during any skill (honoring an
                // explicit "remember this" request from the user without
                // needing a tool-permission escalation).
                if crate::paths::memory_dir().is_dir() {
                    allowed.insert("Read".to_string());
                    allowed.insert("Write".to_string());
                    allowed.insert("Edit".to_string());
                }
                self.inject_http_capability_tools(&mut allowed);
                return Some(allowed);
            }
        }

        let spec = self.spec.as_ref()?;
        if spec.tools.is_empty() {
            return None;
        }
        // Wildcard means unrestricted tool access for this agent.
        if spec.tools.iter().any(|tool| tool.trim() == "*") {
            return None;
        }

        let mut allowed = spec
            .tools
            .iter()
            .filter_map(|tool| {
                // Builtin tools are resolved via canonical_tool_name.
                if let Some(name) = tools::canonical_tool_name(tool) {
                    return Some(name.to_string());
                }
                // Skill tools are recognised by the registry.
                if self.tools.has_skill_tool(tool) {
                    return Some(tool.to_string());
                }
                None
            })
            .collect::<HashSet<String>>();

        // Skill tool is always allowed so the model can discover/invoke skills.
        allowed.insert("Skill".to_string());
        self.inject_http_capability_tools(&mut allowed);

        Some(allowed)
    }

    /// Capability tools (engine-defined contracts like the `Memory_*`
    /// family) are cross-cutting — they live outside any single agent or
    /// slash-invoked skill's declared tool list. Inject every tool from
    /// every active capability into `allowed` when the session's prompt
    /// profile opts in (owner sessions do; consumer / mission sessions
    /// don't). Dispatch routes each call to the active provider's daemon
    /// via `engine::capability_tools`.
    fn inject_http_capability_tools(&self, allowed: &mut HashSet<String>) {
        if !self.prompt_profile.include_memory {
            return;
        }
        for cap in super::capabilities::CAPABILITIES.iter() {
            if !self.tools.active_capabilities.contains(&cap.name) {
                continue;
            }
            for tool in &cap.tools {
                allowed.insert(tool.name.clone());
            }
        }
    }

    pub(crate) fn is_tool_allowed(&self, allowed: &HashSet<String>, requested_tool: &str) -> bool {
        // Builtin tools: check via canonical name.
        if let Some(canonical) = tools::canonical_tool_name(requested_tool) {
            return allowed.contains(canonical);
        }
        // Skill tools: check by exact name.
        allowed.contains(requested_tool)
    }

    pub(crate) fn render_loop_breaker_prompt(template: &str, tool: &str) -> String {
        crate::prompts::PromptStore::substitute(template, &[("tool", tool)])
    }

}

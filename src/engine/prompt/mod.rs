//! System-prompt assembly + prompt-shape co-location.
//!
//! - `profile` — `PromptProfile` (owner/consumer prompt-set selector).
//! - `core_block` — renders the `tier=core` rows from `ling-mem` into
//!   the system prompt's always-on core block + owns the shared
//!   `RECONCILE_FOOTER` instruction string.
//!
//! This submodule is purely organizational — prompt assembly is an
//! internal engine concern, not an extension-type contract like
//! `engine::skill` / `engine::agent` / `engine::mission`.

pub mod core_block;
pub mod profile;

use super::types::*;
use crate::engine::tools;
use crate::message::ChatMessage;
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

/// Best-effort IANA timezone name (e.g. "America/Halifax") from the
/// `/etc/localtime` symlink. Falls back to "unknown" where it can't be read.
/// Stable for the process lifetime, so it's safe in the cached prompt prefix.
fn local_timezone() -> String {
    static TZ: OnceLock<String> = OnceLock::new();
    TZ.get_or_init(|| {
        if let Ok(target) = std::fs::read_link("/etc/localtime") {
            let s = target.to_string_lossy();
            if let Some(idx) = s.find("zoneinfo/") {
                let tz = (&s[idx + "zoneinfo/".len()..]).trim_matches('/');
                if !tz.is_empty() {
                    return tz.to_string();
                }
            }
        }
        "unknown".into()
    })
    .clone()
}

/// Best-effort BCP-47-ish locale (e.g. "en-CA") from the environment.
/// Stable for the process lifetime.
fn locale_tag() -> String {
    static LOCALE: OnceLock<String> = OnceLock::new();
    LOCALE
        .get_or_init(|| {
            for key in ["LC_ALL", "LC_CTYPE", "LANG"] {
                if let Ok(val) = std::env::var(key) {
                    let base = val.split('.').next().unwrap_or("").trim();
                    if !base.is_empty() && base != "C" && base != "POSIX" {
                        return base.replace('_', "-");
                    }
                }
            }
            "unknown".into()
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

        // Hoist the first paragraph of the spec body (typically "You are X — <short
        // self-description>") into the ## Identity block. Keeps the agent's name
        // alive in app-skill / consumer / mission sessions where the rest of the
        // body is stripped, and labels personality traits with a section header
        // for scan/debug clarity.
        let spec_body_full = self.spec_system_prompt.as_deref().map(str::trim).unwrap_or("");
        let (identity_preface, body_rest) = {
            let (head, tail) = spec_body_full.split_once("\n\n").unwrap_or((spec_body_full, ""));
            let head_trim = head.trim();
            if head_trim.is_empty() || head_trim.len() > 300 {
                ("", spec_body_full)
            } else {
                (head_trim, tail.trim_start())
            }
        };

        let identity_block = match (identity_preface.is_empty(), personality.is_empty()) {
            (true, true) => String::new(),
            (false, true) => format!("## Identity\n\n{}", identity_preface),
            (true, false) => format!("## Identity\n\n{}", personality.trim()),
            (false, false) => format!("## Identity\n\n{}\n\n{}", identity_preface, personality.trim()),
        };

        // Mission frame: identity leads, then the agent's full spec body,
        // then the mission body layered on top. The spec body is the
        // agent's doctrine — for a custom mission agent (e.g. `memory`,
        // whose body carries the judgment rules and status-line format)
        // stripping it leaves the mission pointing at instructions the
        // model never sees (the 2026-07-06 dream run failure).
        if let Some(mission) = &self.active_mission {
            let resolved = if let Some(ref dir) = mission.mission_dir {
                mission.body.replace("$MISSION_DIR", &dir.to_string_lossy())
            } else {
                mission.body.clone()
            };
            let parts: Vec<String> = [identity_block, body_rest.to_string(), resolved]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect();
            return parts.join("\n\n");
        }

        let body = if is_app_skill || self.prompt_profile.consumer_frame {
            // App skills: skill content becomes the primary prompt.
            // Consumer sessions: agent spec body describes owner capabilities
            // (coding, delegation, file editing) that consumers don't have.
            // Skip it — the consumer frame in build_stable_system_content
            // provides appropriate instructions.
            String::new()
        } else if body_rest.is_empty() && identity_preface.is_empty() {
            self.prompt_store
                .render_or_fallback(keys::SYSTEM_FALLBACK_IDENTITY, &[])
        } else {
            body_rest.to_string()
        };

        let mut prompt = match (identity_block.is_empty(), body.is_empty()) {
            (true, true) => String::new(),
            (false, true) => identity_block,
            (true, false) => body,
            (false, false) => format!("{}\n\n{}", identity_block, body),
        };

        // Don't list available skills for app skill sessions — the model
        // should focus entirely on the active skill.
        if !is_app_skill && !self.available_skills_metadata.is_empty() {
            // Filter by consumer_allowed_skills when in consumer mode.
            let skills: Vec<&(String, String, bool)> = match &self.cfg.consumer_allowed_skills {
                Some(allowed) => self.available_skills_metadata.iter()
                    .filter(|(name, _, _)| allowed.contains(name))
                    .collect(),
                None => self.available_skills_metadata.iter().collect(),
            };
            if !skills.is_empty() {
                prompt.push_str(&self.prompt_store.render_or_fallback(
                    keys::SYSTEM_SKILLS_HEADER,
                    &[],
                ));
                for (name, description, is_app) in skills {
                    // Mark app skills so an agent (Yinyue) knows which are routable
                    // apps it can hand requests to via agent_chat's `app` target.
                    let display = if *is_app { format!("{name} (app)") } else { name.clone() };
                    prompt.push_str(&self.prompt_store.render_or_fallback(
                        keys::SYSTEM_SKILL_ENTRY,
                        &[("name", display.as_str()), ("description", description.as_str())],
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

        // Note: the `active_mission` branch lives at the top of this
        // function — when a mission is active, the body short-circuits
        // the whole agent-persona path. We don't re-inject it here.

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
            let timezone = local_timezone();
            let locale = locale_tag();
            stable.push_str(&self.prompt_store.render_or_fallback(
                keys::SYSTEM_ENVIRONMENT_BLOCK,
                &[
                    ("platform", std::env::consts::OS),
                    ("os_version", &os_version),
                    ("timezone", &timezone),
                    ("locale", &locale),
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
        // Core = `tier=core` rows from the memory store, inlined
        // unconditionally (no similarity gate). Pulled via the `ling-mem`
        // CLI in `core_block::load_core`. Empty / unreachable store ⇒
        // the bootstrap block fires instead, telling the model how to
        // populate core. Semantic retrieval over the rest of the store
        // reaches the model through the built-in `Memory_query` /
        // `Memory_write` tools (see `engine/tools/memory_tool.rs`) —
        // not through here.
        tracing::info!(
            "prompt build: include_memory={} active_mission={} agent={:?}",
            self.prompt_profile.include_memory,
            self.active_mission.is_some(),
            self.spec.as_ref().map(|s| s.name.clone()),
        );
        if self.prompt_profile.include_memory {
            // Head differs by whether the store has `tier=core` rows; the
            // shared tail (save triggers, retrieval-visibility, usage rules)
            // is one fragment so the two heads can't drift apart.
            match core_block::load_core() {
                Some(c) => stable.push_str(&self.prompt_store.render_or_fallback(
                    keys::CORE_MEMORY_BLOCK,
                    &[("core_facts", &c.facts)],
                )),
                None => stable.push_str(&self.prompt_store.render_or_fallback(
                    keys::CORE_MEMORY_BLOCK_EMPTY,
                    &[],
                )),
            }
            stable.push_str(&self.prompt_store.render_or_fallback(
                keys::CORE_MEMORY_SHARED,
                &[],
            ));
            // Canonical memory protocol — single source of truth for the
            // read-before-write rule, AskUser shape, tier selection, and
            // tier discipline on resolution. Injected once into every
            // memory-enabled session (`include_memory == true`). All other
            // memory prompt surfaces (agent specs, capability tool
            // descriptions) defer to this block.
            stable.push_str(&self.prompt_store.render_or_fallback(
                keys::MEMORY_PROTOCOL,
                &[],
            ));
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

        // A conversational companion (e.g. Yinyue) declares none of the "doing"
        // tools — no files, code, shell, delegation, or planning. It gets a lean
        // prompt and a bare task message (not the coding-framed response format
        // and autonomous-loop bootstrap), so it talks like a person, not a dev agent.
        // Mission runs are never conversational, whatever their tool set: the
        // lean block's "just reply / never end with a status line" voice
        // countermands a mission's turn protocol (observed derailing the
        // dream mission's memory agent, whose tools are Memory-only).
        let conversational = self.active_mission.is_none()
            && !["Read", "Write", "Edit", "Bash", "Grep", "Glob", "Task", "EnterPlanMode", "UpdatePlan"]
                .iter()
                .any(|t| allowed_tools.as_ref().map_or(true, |s| s.contains(*t)));

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
                if conversational {
                    if let Some(lean) = self
                        .prompt_store
                        .get(crate::prompts::keys::RESPONSE_FORMAT_NATIVE_LEAN)
                    {
                        system.push_str("\n\n");
                        system.push_str(lean);
                        if tool_allowed("AskUser") {
                            if let Some(b) = self.prompt_store.get(
                                crate::prompts::keys::RESPONSE_FORMAT_NATIVE_ASKUSER_BULLET,
                            ) {
                                system.push_str(b);
                            }
                        }
                    }
                } else if let Some(base) = self.prompt_store.get(crate::prompts::RESPONSE_FORMAT_NATIVE) {
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

        // The periodic "check whether the recent exchange produced anything
        // durable" nudge that used to fire here has been deleted — the
        // canonical Memory protocol block (`[memory_protocol]` in
        // system-prompt.toml) is already injected into every memory-enabled
        // session, and the nightly dream mission is the offline backstop.
        // A second nudge layer was redundant.

        for obs in &self.observations {
            messages.push(ChatMessage::new("user", self.observation_for_model(obs)));
        }

        // Provide workspace info + task (last user message).
        // Owner gets full workspace listing; consumer gets task only.
        let task_content = if conversational {
            // A companion's "task" is just what the person said — no autonomous-
            // loop framing, no workspace listing, no "explore the codebase / emit
            // a done action" coda (all of which make her sound like a coding agent).
            task.to_string()
        } else if self.prompt_profile.include_workspace_listing {
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
                // Core memory is curated via `Memory_write({tier:"core"})`,
                // not file edits, so the previous auto-grant of
                // Read/Write/Edit on `~/.linggen/memory/` is no longer
                // justified and has been removed. Skills that legitimately
                // need filesystem access must declare it through their own
                // permission surface.
                self.inject_memory_tools(&mut allowed);
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
        self.inject_memory_tools(&mut allowed);

        Some(allowed)
    }

    /// Memory tools (`Memory_query` / `Memory_write`) are cross-cutting
    /// — they live outside any single agent's declared tool list, so
    /// owner sessions get them auto-injected. Mission / consumer
    /// sessions opt out via `prompt_profile.include_memory == false`.
    /// Memory_* are plain built-in tools that talk HTTP to ling-mem; see
    /// `engine/tools/memory_tool.rs`.
    fn inject_memory_tools(&self, allowed: &mut HashSet<String>) {
        if !self.prompt_profile.include_memory {
            return;
        }
        allowed.insert("Memory_query".to_string());
        allowed.insert("Memory_write".to_string());
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

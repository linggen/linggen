use super::skill_tool::{SkillToolDef, SkillToolKind};
use super::tools::{self, ToolCall, ToolResult, Tools};
use crate::engine::agent::AgentManager;
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::debug;

#[derive(Clone)]
pub struct ToolRegistry {
    pub builtins: Tools,
    pub skill_tools: HashMap<String, SkillToolDef>,
}

impl ToolRegistry {
    pub fn new(builtins: Tools) -> Self {
        Self {
            builtins,
            skill_tools: HashMap::new(),
        }
    }

    pub async fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        // 1. Built-in engine tools (Read, Edit, Bash, Memory_query,
        //    Memory_write, ...). Memory_* used to be routed through the
        //    `memory` capability layer; they're now plain built-ins.
        if tools::canonical_tool_name(&call.tool).is_some() {
            return self.builtins.execute(call).await;
        }

        // 2. Skill-unique tools (shell `cmd`, HTTP `endpoint`, or data
        //    tool). Schema + dispatch target live on the SkillToolDef.
        if let Some(skill_tool) = self.skill_tools.get(&call.tool) {
            debug!(
                "Skill tool: {} args={}",
                call.tool,
                tools::summarize_tool_args(&call.tool, &call.args)
            );
            return match skill_tool.kind() {
                SkillToolKind::Http => {
                    self.dispatch_via_skill_http(&skill_tool.name, &call.args).await
                }
                _ => skill_tool.execute(&call.args, &self.builtins.cwd()),
            };
        }

        anyhow::bail!("unknown tool: {}", call.tool)
    }

    /// Stub for skill-declared HTTP tools. No production skill uses the
    /// `Http` kind today; the capability-routing path was removed when
    /// memory moved to built-in tools. Re-add via a direct reqwest POST
    /// helper here if a real consumer appears, or drop the kind.
    async fn dispatch_via_skill_http(&self, name: &str, args: &Value) -> Result<ToolResult> {
        let _ = args;
        Err(anyhow!(
            "Skill HTTP tool '{name}' dispatch is not implemented — \
             re-add a direct POST helper if a skill needs HTTP-backed tools."
        ))
    }

    /// Resolve a tool name to its canonical form. Returns the name if it
    /// is a known built-in (now including Memory_*) or a registered
    /// skill tool.
    pub fn canonical_tool_name<'a>(&self, tool: &'a str) -> Option<&'a str> {
        if tools::canonical_tool_name(tool).is_some() {
            return Some(tool);
        }
        if self.skill_tools.contains_key(tool) {
            return Some(tool);
        }
        None
    }

    /// Returns true if `tool` is a registered skill tool.
    pub fn has_skill_tool(&self, tool: &str) -> bool {
        self.skill_tools.contains_key(tool)
    }

    /// Returns true if `tool` is a skill data tool — a tool with neither
    /// a `cmd` nor an `endpoint`, which just echoes its args back as a
    /// JSON content block (e.g. the built-in `PageUpdate` for app skills).
    /// Data tools touch no files, run no commands, and pose no privilege
    /// risk, so the permission layer should skip its path/tier check
    /// entirely for them. HTTP tools are NOT data tools — they still go
    /// through permission checks.
    pub fn is_skill_data_tool(&self, tool: &str) -> bool {
        self.skill_tools
            .get(tool)
            .map(|def| def.kind() == SkillToolKind::Data)
            .unwrap_or(false)
    }

    /// `true` if `name` passes the allowed-tools filter (or no filter set).
    fn is_allowed(allowed: Option<&HashSet<String>>, name: &str) -> bool {
        match allowed {
            // Wildcard (`*`): everything EXCEPT pet-scoped tools (only an agent
            // that lists them explicitly gets those).
            None => !tools::is_pet_scoped(name),
            Some(set) => set.contains(name),
        }
    }

    /// Merge built-in and skill tool schemas, filtered by the allowed
    /// set. Built-ins now include Memory_query / Memory_write directly
    /// (the old capability layer is gone — see PR2).
    pub fn tool_schema_json(&self, allowed_tools: Option<&HashSet<String>>) -> String {
        let mut tools_arr = tools::full_tool_schema_entries();
        tools_arr.retain(|entry| {
            entry
                .get("name")
                .and_then(|v| v.as_str())
                .map(|name| Self::is_allowed(allowed_tools, name))
                .unwrap_or(false)
        });

        for (name, def) in &self.skill_tools {
            if !Self::is_allowed(allowed_tools, name) {
                continue;
            }
            tools_arr.push(def.to_schema_json());
        }

        serde_json::json!({ "tools": tools_arr }).to_string()
    }

    /// Build OpenAI-compatible tool definitions for native function
    /// calling. Built-ins (incl. Memory_*) + skill-unique tools, filtered
    /// by the allowed set.
    pub fn oai_tool_definitions(&self, allowed: Option<&HashSet<String>>) -> Vec<serde_json::Value> {
        let mut defs = tools::json_schema::oai_tool_definitions(allowed);

        for (name, def) in &self.skill_tools {
            if !Self::is_allowed(allowed, name) {
                continue;
            }
            defs.push(def.to_oai_schema());
        }
        defs
    }

    pub fn register_skill_tool(&mut self, tool: SkillToolDef) {
        self.skill_tools.insert(tool.name.clone(), tool);
    }

    // --- Passthrough methods to builtins ---

    pub fn set_context(
        &mut self,
        manager: Arc<AgentManager>,
        agent_id: String,
    ) {
        self.builtins.set_context(manager, agent_id);
    }

    pub fn set_run_id(&mut self, run_id: Option<String>) {
        self.builtins.set_run_id(run_id);
    }

    pub fn get_manager(&self) -> Option<Arc<AgentManager>> {
        self.builtins.get_manager()
    }

    pub fn set_ask_user_bridge(&mut self, bridge: std::sync::Arc<tools::AskUserBridge>) {
        self.builtins.set_ask_user_bridge(bridge);
    }

    pub fn ask_user_bridge(&self) -> Option<&std::sync::Arc<tools::AskUserBridge>> {
        self.builtins.ask_user_bridge()
    }
}

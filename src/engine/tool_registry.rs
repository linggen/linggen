use super::skill_tool::SkillToolDef;
use super::tools::{self, ToolCall, ToolResult, Tools};
use crate::agent_manager::AgentManager;
use crate::config::AgentPolicy;
use anyhow::Result;
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

    pub fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        // Builtin tools are dispatched to the existing Tools implementation.
        if tools::canonical_tool_name(&call.tool).is_some() {
            return self.builtins.execute(call);
        }

        // Skill tools are dispatched via SkillToolDef.
        if let Some(skill_tool) = self.skill_tools.get(&call.tool) {
            debug!("Skill tool: {} args={}", call.tool, tools::summarize_tool_args(&call.tool, &call.args));
            return skill_tool.execute(&call.args, self.builtins.workspace_root());
        }

        anyhow::bail!("unknown tool: {}", call.tool)
    }

    /// Resolve a tool name to its canonical form.
    /// Returns the name if it is a known builtin or registered skill tool.
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

    /// Merge builtin and skill tool schemas, filtered by the allowed set.
    pub fn tool_schema_json(&self, allowed_tools: Option<&HashSet<String>>) -> String {
        let mut tools_arr = tools::full_tool_schema_entries();

        // Filter builtins by allowed set.
        if let Some(allowed) = allowed_tools {
            tools_arr.retain(|entry| {
                entry
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(|name| allowed.contains(name))
                    .unwrap_or(false)
            });
        }

        // Append skill tool schemas.
        for (name, def) in &self.skill_tools {
            if let Some(allowed) = allowed_tools {
                if !allowed.contains(name) {
                    continue;
                }
            }
            tools_arr.push(def.to_schema_json());
        }

        serde_json::json!({ "tools": tools_arr }).to_string()
    }

    /// Build OpenAI-compatible tool definitions for native function calling.
    /// Merges builtin tool schemas with skill tool schemas, filtered by allowed set.
    pub fn oai_tool_definitions(&self, allowed: Option<&HashSet<String>>) -> Vec<serde_json::Value> {
        let mut defs = tools::json_schema::oai_tool_definitions(allowed);
        for (name, def) in &self.skill_tools {
            if let Some(set) = allowed {
                if !set.contains(name) {
                    continue;
                }
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

    pub fn set_policy(&mut self, policy: Option<AgentPolicy>) {
        self.builtins.set_policy(policy);
    }

    pub fn set_run_id(&mut self, run_id: Option<String>) {
        self.builtins.set_run_id(run_id);
    }

    pub fn get_manager(&self) -> Option<Arc<AgentManager>> {
        self.builtins.get_manager()
    }

    pub fn set_memory_dir(&mut self, dir: PathBuf) {
        self.builtins.set_memory_dir(dir);
    }

    pub fn memory_dir(&self) -> Option<&PathBuf> {
        self.builtins.memory_dir()
    }

    pub fn set_ask_user_bridge(&mut self, bridge: std::sync::Arc<tools::AskUserBridge>) {
        self.builtins.set_ask_user_bridge(bridge);
    }

    pub fn ask_user_bridge(&self) -> Option<&std::sync::Arc<tools::AskUserBridge>> {
        self.builtins.ask_user_bridge()
    }
}

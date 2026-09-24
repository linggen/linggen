//! AppTool — the companion reads an app's state through a tool the app
//! offers her (`pet: true` in its SKILL.md). The gate is the engine's
//! (`engine::skill::tools::run_for_pet`), not the prompt's.

use super::super::{ToolCall, ToolResult, Tools};
use super::Tool;
use crate::engine::permission::PermissionMode;
use crate::engine::skill::tools::{pet_tools, run_for_pet, Refusal};
use crate::engine::skill::Skill;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

#[derive(serde::Deserialize)]
struct AppToolArgs {
    #[serde(default)]
    app: Option<String>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    args: Option<Value>,
}

pub struct AppTool;
#[async_trait]
impl Tool for AppTool {
    fn name(&self) -> &'static str {
        "AppTool"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["app_tool"]
    }
    fn description(&self) -> &'static str {
        "Read an app's state through a tool the app offers you. Call with no \
         `tool` to list what the apps offer (app, tool, description, args); \
         then call with app + tool + args."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "app": { "type": "string", "description": "The app (skill) name." },
                "tool": {
                    "type": "string",
                    "description": "The tool to call. Omit to list the tools apps offer you."
                },
                "args": { "type": "object", "description": "The tool's args." }
            }
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "AppTool",
            "args": {"app": "string?", "tool": "string?", "args": "object?"},
            "returns": "the tool's output, or the list of tools apps offer you",
            "notes": "No `tool`: list what apps offer you. Only tools an app offers you run."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: AppToolArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for AppTool: {e}"))?;
        let manager = tools
            .get_manager()
            .ok_or_else(|| anyhow::anyhow!("AppTool: no apps to reach here"))?;
        let skills = match args.app.as_deref() {
            Some(app) => vec![manager
                .skills
                .get_skill(app)
                .await
                .ok_or_else(|| anyhow::anyhow!(Refusal::no_skill(app).to_string()))?],
            None => manager.skills.list_skills().await,
        };
        let Some(tool) = args.tool.as_deref() else {
            return Ok(ToolResult::Success(listing(&skills)));
        };
        let [skill] = skills.as_slice() else {
            anyhow::bail!("AppTool: name the `app` whose tool '{tool}' to call");
        };
        let tool_args = args.args.unwrap_or_else(|| json!({}));
        if !tool_args.is_object() {
            anyhow::bail!("AppTool: `args` is the tool's args, as an object");
        }
        run_for_pet(skill, tool, &tool_args)
            .await
            .map_err(|r| anyhow::anyhow!(r.to_string()))
    }
}

/// Every tool the given apps offer the companion, as JSON lines.
fn listing(skills: &[Skill]) -> String {
    let rows: Vec<String> = skills
        .iter()
        .flat_map(|s| pet_tools(s).map(move |t| (s, t)))
        .map(|(s, t)| {
            let entry = t.to_schema_json();
            json!({
                "app": s.name,
                "tool": t.name,
                "description": t.description,
                "args": entry["args"],
            })
            .to_string()
        })
        .collect();
    if rows.is_empty() {
        return "No app offers you a tool right now.".to_string();
    }
    rows.join("\n")
}

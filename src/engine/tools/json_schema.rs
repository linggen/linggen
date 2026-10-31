//! Build OpenAI-compatible tool definitions for the model's
//! function-calling `tools` API parameter.
//!
//! The list comes from two sources, in order:
//! 1. The built-in tool registry ([`super::builtin`]) — each registered
//!    `Tool` whose `model_facing()` is true contributes name, description,
//!    and args schema.
//! 2. Plan-mode tools (`EnterPlanMode`, `ExitPlanMode`, `UpdatePlan`)
//!    which are parsed as `ModelAction`s in `actions.rs` and don't have
//!    `Tool` impls — listed inline below.
//!
//! Memory_* and other skill-declared HTTP tools come from each skill's
//! SKILL.md `tools:` block; the `ToolRegistry` renders them via
//! `SkillToolDef::to_oai_schema()` when composing the final tool list.

use super::builtin;
use serde_json::{json, Value};
use std::collections::HashSet;

/// All tool definitions, optionally filtered to the `allowed` set.
pub fn oai_tool_definitions(allowed: Option<&HashSet<String>>) -> Vec<Value> {
    let all: Vec<Value> = builtin::model_facing_args_schemas()
        .into_iter()
        .map(|(name, description, parameters)| tool_def(&name, &description, parameters))
        .chain(plan_mode_schemas())
        // Tools discovered from configured MCP servers. They are not `Tool`
        // impls — they are found at runtime, so they join here, at the same
        // seam plan mode uses.
        .chain(mcp_schemas())
        .collect();

    match allowed {
        Some(set) => all
            .into_iter()
            .filter(|def| {
                def.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .map(|name| set.contains(name))
                    .unwrap_or(false)
            })
            .collect(),
        // Wildcard (`*`): everything EXCEPT pet-scoped tools (Express) — a worker
        // agent shouldn't drive the avatar; only an explicit lister (Yinyue) gets it.
        None => all
            .into_iter()
            .filter(|def| {
                def.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .map(|name| !super::is_pet_scoped(name))
                    .unwrap_or(true)
            })
            .collect(),
    }
}

/// Tool schemas discovered from MCP servers, server-qualified.
fn mcp_schemas() -> impl Iterator<Item = Value> {
    crate::mcp_client::registry()
        .advertised()
        .into_iter()
        .map(|t| tool_def(&t.qualified, &t.description, t.input_schema))
}

/// Plan-mode tool schemas. Not real `Tool` impls — they're parsed as
/// `ModelAction`s in `actions.rs` and dispatched specially by the loop.
fn plan_mode_schemas() -> impl Iterator<Item = Value> {
    [
        tool_def(
            "ExitPlanMode",
            "Submit your plan for user approval. You MUST include the full plan text in the plan_text parameter. The system will prompt the user to approve, reject, or give feedback — do NOT ask for confirmation in your response text. Just call this tool when the plan is ready.",
            json!({
                "type": "object",
                "properties": {
                    "plan_text": {
                        "type": "string",
                        "description": "The full markdown plan text to submit for user review. Include all steps, file paths, and implementation details."
                    }
                },
                "required": ["plan_text"]
            }),
        ),
        tool_def(
            "EnterPlanMode",
            "Enter plan mode to research and produce an implementation plan for user approval. Only for work that will change things: writing code, writing docs, editing config or files. Use it proactively before a non-trivial implementation: a new feature, a change to existing behavior, several valid approaches, an architectural choice, edits likely to touch more than 2-3 files, or unclear requirements. Skip it for simple work: a few-line fix, one function with clear requirements, or a task the user has spelled out in detail. Never for questions, research, comparisons, recommendations, shopping, or advice — answer those directly. In plan mode you have read-only tools until you call ExitPlanMode.",
            json!({
                "type": "object",
                "properties": {
                    "reason": {
                        "type": "string",
                        "description": "Why plan mode is needed"
                    }
                },
                "required": []
            }),
        ),
        tool_def(
            "UpdatePlan",
            "Track execution progress on a task of 3 or more distinct steps that changes files (code, docs, config); use it proactively so the user sees what is done and left: items is the checklist, plan_text optionally updates the approved plan. Skip it for a single simple step and for answering questions. Not available while planning — submit a plan with ExitPlanMode.",
            json!({
                "type": "object",
                "properties": {
                    "plan_text": {
                        "type": "string",
                        "description": "Detailed markdown plan text with implementation steps, file paths, and code snippets. If omitted, existing plan_text is preserved."
                    },
                    "items": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": {"type": "string"},
                                "title": {"type": "string"},
                                "status": {
                                    "type": "string",
                                    "enum": ["pending", "in_progress", "completed"]
                                }
                            },
                            "required": ["id", "title", "status"]
                        }
                    }
                },
                "required": []
            }),
        ),
    ]
    .into_iter()
}

fn tool_def(name: &str, description: &str, parameters: Value) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oai_tool_definitions_returns_all() {
        let defs = oai_tool_definitions(None);
        assert!(
            defs.len() >= 10,
            "expected at least 10 tool definitions, got {}",
            defs.len()
        );
        for def in &defs {
            assert_eq!(def["type"], "function");
            assert!(def["function"]["name"].is_string());
            assert!(def["function"]["description"].is_string());
            assert!(def["function"]["parameters"].is_object());
        }
    }

    #[test]
    fn test_oai_tool_definitions_filters_by_allowed() {
        let mut allowed = HashSet::new();
        allowed.insert("Read".to_string());
        allowed.insert("Write".to_string());
        let defs = oai_tool_definitions(Some(&allowed));
        assert_eq!(defs.len(), 2);
        let names: Vec<&str> = defs
            .iter()
            .filter_map(|d| d["function"]["name"].as_str())
            .collect();
        assert!(names.contains(&"Read"));
        assert!(names.contains(&"Write"));
    }

    #[test]
    fn test_read_tool_schema_has_required_path() {
        let defs = oai_tool_definitions(None);
        let read = defs
            .iter()
            .find(|d| d["function"]["name"] == "Read")
            .unwrap();
        let required = read["function"]["parameters"]["required"]
            .as_array()
            .unwrap();
        assert!(required.iter().any(|v| v == "path"));
    }
}

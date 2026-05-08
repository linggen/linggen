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
        None => all,
    }
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
            "Enter plan mode to research and produce a detailed implementation plan for user approval. Use this when the user asks you to 'plan', 'design', or 'propose' something, or when a task is complex enough to need upfront exploration before making changes. In plan mode you are restricted to read-only tools until you call ExitPlanMode.",
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
            "Update the plan content and/or progress checklist. Use plan_text for the detailed implementation plan (markdown with file paths, code snippets, explanations). Use items for the progress checklist. Both can be provided together.",
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
        assert!(defs.len() >= 10, "expected at least 10 tool definitions, got {}", defs.len());
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
        let read = defs.iter().find(|d| d["function"]["name"] == "Read").unwrap();
        let required = read["function"]["parameters"]["required"]
            .as_array()
            .unwrap();
        assert!(required.iter().any(|v| v == "path"));
    }
}

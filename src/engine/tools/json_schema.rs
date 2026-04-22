use serde_json::{json, Value};
use std::collections::HashSet;

/// Build OpenAI-compatible tool definitions for all builtin tools.
/// When `allowed` is `Some`, only tools in the set are included.
pub fn oai_tool_definitions(allowed: Option<&HashSet<String>>) -> Vec<Value> {
    let all = builtin_tool_schemas();
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

fn builtin_tool_schemas() -> Vec<Value> {
    vec![
        tool_def(
            "Glob",
            "Find files by glob pattern. Returns matching file paths sorted by modification time.",
            json!({
                "type": "object",
                "properties": {
                    "globs": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Glob patterns to match (e.g. [\"**/*.rs\", \"src/**/*.ts\"])"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum number of results to return"
                    }
                },
                "required": ["globs"]
            }),
        ),
        tool_def(
            "Read",
            "Read a file's contents. Path can be relative (resolved from workspace root) or absolute. Always read a file before modifying it.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path to read (relative to workspace root, or absolute)"
                    },
                    "max_bytes": {
                        "type": "integer",
                        "description": "Maximum bytes to read (default: entire file)"
                    },
                    "line_range": {
                        "type": "array",
                        "items": {"type": "integer"},
                        "minItems": 2,
                        "maxItems": 2,
                        "description": "Line range [start, end] (1-based, inclusive)"
                    }
                },
                "required": ["path"]
            }),
        ),
        tool_def(
            "Grep",
            "Search file contents using regex. Returns matching lines with file path, line number, and snippet.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Regex pattern to search for"
                    },
                    "globs": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "File glob patterns to search within (e.g. [\"**/*.rs\"])"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum number of matches to return"
                    }
                },
                "required": ["query"]
            }),
        ),
        tool_def(
            "Write",
            "Write content to a file (creates or overwrites). Prefer Edit for existing files. Path is relative to workspace root.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path to write (relative to workspace root)"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write to the file"
                    }
                },
                "required": ["path", "content"]
            }),
        ),
        tool_def(
            "Edit",
            "Apply an exact string replacement in a file. Prefer this over Write for existing files. Read the file first.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path to edit (relative to workspace root)"
                    },
                    "old_string": {
                        "type": "string",
                        "description": "Exact string to find and replace"
                    },
                    "new_string": {
                        "type": "string",
                        "description": "Replacement string"
                    },
                    "replace_all": {
                        "type": "boolean",
                        "description": "Replace all occurrences (default: false, replaces first only)"
                    }
                },
                "required": ["path", "old_string", "new_string"]
            }),
        ),
        tool_def(
            "Bash",
            "Run a shell command via sh -c. Working directory persists across calls (cd is remembered). Use for build, test, git, and other commands that require shell execution. Prefer dedicated tools (Read, Glob, Grep) over Bash equivalents.",
            json!({
                "type": "object",
                "properties": {
                    "cmd": {
                        "type": "string",
                        "description": "Shell command to execute"
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "description": "Timeout in milliseconds (default: 30000)"
                    }
                },
                "required": ["cmd"]
            }),
        ),
        tool_def(
            "capture_screenshot",
            "Capture a screenshot of a URL.",
            json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "URL to capture"
                    },
                    "delay_ms": {
                        "type": "integer",
                        "description": "Delay before capture in milliseconds"
                    }
                },
                "required": ["url"]
            }),
        ),
        tool_def(
            "Task",
            "Delegate a task to another agent. Send a specific task description with clear scope and expected output.",
            json!({
                "type": "object",
                "properties": {
                    "target_agent_id": {
                        "type": "string",
                        "description": "ID of the agent to delegate to"
                    },
                    "task": {
                        "type": "string",
                        "description": "Task description for the target agent"
                    }
                },
                "required": ["target_agent_id", "task"]
            }),
        ),
        tool_def(
            "WebSearch",
            "Search the web via DuckDuckGo. Returns titles, URLs, and snippets.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum results (default: 5, max: 10)"
                    }
                },
                "required": ["query"]
            }),
        ),
        tool_def(
            "WebFetch",
            "Fetch a URL and return its content as text. HTML tags are stripped. Default max 100KB.",
            json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "URL to fetch"
                    },
                    "max_bytes": {
                        "type": "integer",
                        "description": "Maximum bytes to return (default: 100000)"
                    }
                },
                "required": ["url"]
            }),
        ),
        tool_def(
            "Skill",
            "Invoke a skill by name. Returns the skill's full instructions. Use to discover and run installed skills.",
            json!({
                "type": "object",
                "properties": {
                    "skill": {
                        "type": "string",
                        "description": "Skill name to invoke"
                    },
                    "args": {
                        "type": "string",
                        "description": "Optional arguments for the skill"
                    }
                },
                "required": ["skill"]
            }),
        ),
        tool_def(
            "AskUser",
            "Ask the user 1-4 structured questions with 2-6 options each. User can always type custom text. Blocks until response (5 min timeout).",
            json!({
                "type": "object",
                "properties": {
                    "questions": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "question": {"type": "string"},
                                "header": {"type": "string"},
                                "options": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "label": {"type": "string"},
                                            "description": {"type": "string"}
                                        },
                                        "required": ["label"]
                                    }
                                },
                                "multi_select": {"type": "boolean"}
                            },
                            "required": ["question", "header", "options"]
                        }
                    }
                },
                "required": ["questions"]
            }),
        ),
        tool_def(
            "RunApp",
            "Launch an app-enabled skill. The skill must have an 'app' config with a launcher (web/bash/url). For web apps, returns the URL to open in the UI.",
            json!({
                "type": "object",
                "properties": {
                    "skill": {
                        "type": "string",
                        "description": "Name of the skill to launch"
                    },
                    "args": {
                        "type": "string",
                        "description": "Optional arguments for the skill"
                    }
                },
                "required": ["skill"]
            }),
        ),
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
        // Memory_* and any other skill-declared HTTP tools come from their
        // skill's SKILL.md `tools:` block, not from this hardcoded table.
        // The ToolRegistry renders them via `SkillToolDef::to_oai_schema()`
        // when building the native tool list for the model.
    ]
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
        // Check all have the required structure
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
        let names: Vec<&str> = defs.iter()
            .filter_map(|d| d["function"]["name"].as_str())
            .collect();
        assert!(names.contains(&"Read"));
        assert!(names.contains(&"Write"));
    }

    #[test]
    fn test_tool_def_structure() {
        let def = tool_def("Test", "A test tool", json!({
            "type": "object",
            "properties": {"arg1": {"type": "string"}},
            "required": ["arg1"]
        }));
        assert_eq!(def["type"], "function");
        assert_eq!(def["function"]["name"], "Test");
        assert_eq!(def["function"]["description"], "A test tool");
        assert_eq!(def["function"]["parameters"]["properties"]["arg1"]["type"], "string");
    }

    #[test]
    fn test_read_tool_schema_has_required_path() {
        let defs = oai_tool_definitions(None);
        let read = defs.iter().find(|d| d["function"]["name"] == "Read").unwrap();
        let required = read["function"]["parameters"]["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "path"));
    }

    // Memory tool schemas are now declared in skills/memory/SKILL.md
    // and rendered via SkillToolDef::to_oai_schema. Schema integrity is
    // exercised by skill-parsing tests in the skills module.
}

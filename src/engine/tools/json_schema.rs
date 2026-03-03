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
            "Read a file's contents. Path is relative to workspace root. Always read a file before modifying it.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path to read (relative to workspace root)"
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
            "Run a shell command via sh -c. Use for build, test, git, and other commands that require shell execution. Prefer dedicated tools (Read, Glob, Grep) over Bash equivalents.",
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
            "Ask the user 1-4 structured questions with 2-4 options each. User can always type custom text. Blocks until response (5 min timeout).",
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
            "ExitPlanMode",
            "Signal that your plan is complete and ready for user review. Call after researching and writing your plan.",
            json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        ),
        // Non-tool actions promoted to tools for native function calling
        tool_def(
            "Done",
            "Signal task completion with a summary. For conversational replies, just respond with text content instead.",
            json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "Summary of what was accomplished"
                    }
                },
                "required": []
            }),
        ),
        tool_def(
            "EnterPlanMode",
            "Request plan mode for complex tasks that need upfront exploration before implementation.",
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
            "Track progress on multi-step tasks. Update item statuses as you complete each step.",
            json!({
                "type": "object",
                "properties": {
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
                "required": ["items"]
            }),
        ),
        tool_def(
            "Patch",
            "Apply a unified diff patch to the workspace.",
            json!({
                "type": "object",
                "properties": {
                    "diff": {
                        "type": "string",
                        "description": "Unified diff content"
                    }
                },
                "required": ["diff"]
            }),
        ),
        tool_def(
            "FinalizeTask",
            "Finalize a delegated task with a structured task packet.",
            json!({
                "type": "object",
                "properties": {
                    "packet": {
                        "type": "object",
                        "properties": {
                            "title": {"type": "string"},
                            "user_stories": {"type": "array", "items": {"type": "string"}},
                            "acceptance_criteria": {"type": "array", "items": {"type": "string"}},
                            "mermaid_wireframe": {"type": "string"}
                        },
                        "required": ["title", "user_stories", "acceptance_criteria"]
                    }
                },
                "required": ["packet"]
            }),
        ),
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
        assert!(defs.len() >= 13, "expected at least 13 tool definitions, got {}", defs.len());
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
}

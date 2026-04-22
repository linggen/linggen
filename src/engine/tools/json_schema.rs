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
        // ── Memory.* family ─────────────────────────────────────────────
        // Routed to the active `provides: [memory]` skill via
        // engine::memory::dispatch. When no provider is installed, every
        // call returns a clear install-hint error. Argument shapes track
        // linggen-memory/DESIGN.md; drift is a bug to be reconciled there.
        tool_def(
            MEMORY_ADD,
            "Store a new fact in memory. Use for durable, scoped info worth recalling in future unrelated sessions — identity, preferences, decisions with reasoning, failed attempts, symptom-indexed fixes. Not for activity logs or conversation micro-details.",
            json!({
                "type": "object",
                "properties": {
                    "content": {"type": "string", "description": "The fact text. Self-contained; include scoping conditions inline if they matter."},
                    "contexts": {"type": "array", "items": {"type": "string"}, "description": "Scope tags (e.g. [\"code/linggen\", \"trip-japan-2026\"]). Free-form; N:M with facts."},
                    "type": {"type": "string", "enum": ["fact", "preference", "decision", "tried", "fixed", "learned", "built"], "description": "Canonical fact type. See linggen-memory/DESIGN.md."},
                    "outcome": {"type": "string", "enum": ["positive", "negative", "neutral"], "description": "Only meaningful for action-flavored types (tried, fixed)."}
                },
                "required": ["content"]
            }),
        ),
        tool_def(
            MEMORY_GET,
            "Fetch a single fact by id.",
            json!({
                "type": "object",
                "properties": {"id": {"type": "string"}},
                "required": ["id"]
            }),
        ),
        tool_def(
            MEMORY_SEARCH,
            "Semantic search across stored facts. Use when the query is fuzzy or you want relevance ranking; prefer Memory.list for exact-filter browsing.",
            json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "contexts": {"type": "array", "items": {"type": "string"}, "description": "Restrict to these scope tags."},
                    "type": {"type": "string"},
                    "limit": {"type": "integer"}
                },
                "required": ["query"]
            }),
        ),
        tool_def(
            MEMORY_LIST,
            "Browse facts without semantic ranking. Use for deterministic filters: exact context, date range, sort by created_at or occurred_at.",
            json!({
                "type": "object",
                "properties": {
                    "contexts": {"type": "array", "items": {"type": "string"}},
                    "type": {"type": "string"},
                    "since": {"type": "string", "description": "ISO timestamp — inclusive lower bound."},
                    "limit": {"type": "integer"},
                    "sort": {"type": "string", "enum": ["created_at", "occurred_at"]}
                },
                "required": []
            }),
        ),
        tool_def(
            MEMORY_UPDATE,
            "Edit an existing fact by id. Any omitted field is left untouched.",
            json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "content": {"type": "string"},
                    "contexts": {"type": "array", "items": {"type": "string"}},
                    "type": {"type": "string"},
                    "outcome": {"type": "string"}
                },
                "required": ["id"]
            }),
        ),
        tool_def(
            MEMORY_DELETE,
            "Hard-delete a single fact with a tombstone. Irreversible.",
            json!({
                "type": "object",
                "properties": {"id": {"type": "string"}},
                "required": ["id"]
            }),
        ),
        tool_def(
            MEMORY_FORGET,
            "Bulk-delete facts by filter. Use when the user says 'forget everything about X' — contexts + type + older_than narrow the target.",
            json!({
                "type": "object",
                "properties": {
                    "contexts": {"type": "array", "items": {"type": "string"}},
                    "type": {"type": "string"},
                    "older_than": {"type": "string", "description": "ISO timestamp — delete facts older than this."}
                },
                "required": []
            }),
        ),
    ]
}

// Memory.* tool names — mirrored from `tool_helpers::MEMORY_TOOL_NAMES` so
// the native-schema module stays independent of the legacy-schema module
// while staying in sync. Violations of this mirroring are caught by
// `tests::memory_tools_match_canonical_names`.
const MEMORY_ADD: &str = "Memory.add";
const MEMORY_GET: &str = "Memory.get";
const MEMORY_SEARCH: &str = "Memory.search";
const MEMORY_LIST: &str = "Memory.list";
const MEMORY_UPDATE: &str = "Memory.update";
const MEMORY_DELETE: &str = "Memory.delete";
const MEMORY_FORGET: &str = "Memory.forget";

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

    #[test]
    fn test_all_memory_tools_have_native_schemas() {
        let defs = oai_tool_definitions(None);
        let names: std::collections::HashSet<&str> = defs
            .iter()
            .filter_map(|d| d["function"]["name"].as_str())
            .collect();
        for expected in super::super::tool_helpers::MEMORY_TOOL_NAMES {
            assert!(
                names.contains(expected),
                "Memory tool '{expected}' missing from OAI definitions — \
                 native-function-calling models won't see it. Keep \
                 MEMORY_TOOL_NAMES and builtin_tool_schemas in sync.",
            );
        }
    }

    #[test]
    fn test_memory_add_schema_requires_content() {
        let defs = oai_tool_definitions(None);
        let add = defs
            .iter()
            .find(|d| d["function"]["name"] == "Memory.add")
            .expect("Memory.add present");
        let required = add["function"]["parameters"]["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "content"));
    }
}

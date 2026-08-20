use anyhow::Result;
use serde::Deserialize;
use serde_json::de::Deserializer;
use serde_json::Value;

/// The only action a model can produce: a tool call.
/// The loop stops when the model emits no tool calls (CC-aligned).
#[derive(Debug, Deserialize)]
pub struct ModelAction {
    pub tool: String,
    pub args: serde_json::Value,
}

#[allow(dead_code)]
pub fn parse_first_action(raw: &str) -> Result<ModelAction> {
    let trimmed = raw.trim();

    // Fast path: a single clean JSON object.
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(action) = value_to_action(value) {
            return Ok(action);
        }
    }

    // Fallback: models sometimes emit prose + one or more JSON objects.
    // Scan for JSON object starts and return the first valid ModelAction.
    // IMPORTANT: do not strip angle-bracket content here.
    // Rust/TS code in tool args can contain generics like `Option<u64>` and
    // removing `<...>` would corrupt write payloads.
    let cleaned_trimmed = trimmed;

    for (idx, _) in cleaned_trimmed.match_indices('{') {
        let candidate = &cleaned_trimmed[idx..];
        let stream = Deserializer::from_str(candidate).into_iter::<serde_json::Value>();
        for value in stream.flatten() {
            if let Some(action) = value_to_action(value) {
                return Ok(action);
            }
        }
    }

    anyhow::bail!("no valid model action found in response")
}

/// Parse ALL valid model actions from a response that may contain prose + multiple JSON objects.
/// Actions are returned in the order they appear in the response.
pub fn parse_all_actions(raw: &str) -> Result<Vec<ModelAction>> {
    let trimmed = raw.trim();
    let mut actions = Vec::new();
    let mut pos = 0;

    while pos < trimmed.len() {
        let Some(brace_idx) = trimmed[pos..].find('{') else {
            break;
        };
        let abs_idx = pos + brace_idx;
        let candidate = &trimmed[abs_idx..];
        let mut de = Deserializer::from_str(candidate).into_iter::<serde_json::Value>();
        if let Some(Ok(value)) = de.next() {
            let consumed = de.byte_offset();
            if let Some(action) = value_to_action(value) {
                actions.push(action);
            }
            pos = abs_idx + consumed;
        } else {
            pos = abs_idx + 1;
        }
    }

    if actions.is_empty() {
        anyhow::bail!("no valid model action found in response");
    }
    Ok(actions)
}

fn value_to_action(value: serde_json::Value) -> Option<ModelAction> {
    let obj = value.as_object()?;
    let action_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let args = obj
        .get("args")
        .or_else(|| obj.get("tool_args"))
        .cloned()
        .unwrap_or_else(|| {
            // Models sometimes put tool arguments as top-level fields instead of nesting
            // under "args". E.g. {"type":"Task","target_agent_id":"x","task":"y"}
            // Collect all fields except "type", "tool", "name" as an args object.
            let mut inferred = serde_json::Map::new();
            for (k, v) in obj {
                if k != "type" && k != "tool" && k != "name" {
                    inferred.insert(k.clone(), v.clone());
                }
            }
            if inferred.is_empty() {
                serde_json::json!({})
            } else {
                Value::Object(inferred)
            }
        });

    // Check "tool" field first, then "name" — models sometimes emit
    // {"name":"Read","args":{...}} instead of {"type":"tool","tool":"Read","args":{...}}.
    if let Some(tool) = obj
        .get("tool")
        .or_else(|| obj.get("name"))
        .and_then(|v| v.as_str())
    {
        // "done"/"Done" is no longer a special action — ignore it.
        // The loop stops when the model has no tool calls.
        if tool.eq_ignore_ascii_case("done") {
            return None;
        }
        return Some(ModelAction {
            tool: tool.to_string(),
            args,
        });
    }

    // Legacy action type names that map directly to tools.
    // Note: {"type":"tool","tool":"Read"} is already handled above (the "tool" key
    // check at line 102 fires first), so we don't need a "tool" arm here.
    let tool_name = match action_type {
        "Read" | "Grep" | "Write" | "Edit" | "Glob" | "Bash" | "capture_screenshot"
        | "lock_paths" | "unlock_paths" | "Task" | "delegate_to_agent" | "EnterPlanMode"
        | "enter_plan_mode" | "UpdatePlan" | "update_plan" => {
            // Normalize legacy snake_case action types to PascalCase tool names.
            match action_type {
                "enter_plan_mode" => "EnterPlanMode",
                "update_plan" => "UpdatePlan",
                other => other,
            }
        }
        // "done", "patch", "finalize_task" — no longer actions, ignore.
        "done" | "patch" | "finalize_task" => return None,
        // Empty type with text-like fields — not a tool call, ignore.
        "" => {
            // Handle {"done":true} — not a tool call.
            if obj.get("done").and_then(|v| v.as_bool()).unwrap_or(false) {
                return None;
            }
            // {"response":"..."} etc. — not a tool call.
            return None;
        }
        // Unknown type with args present: treat as a skill tool shorthand
        // (e.g. {"type":"my_skill_tool","args":{...}}).
        other if obj.contains_key("args") || obj.contains_key("tool_args") => other,
        _ => return None,
    };

    Some(ModelAction {
        tool: tool_name.to_string(),
        args,
    })
}

/// Try to parse the first complete JSON action from a buffer.
/// Returns the action and the byte offset past its end in the buffer.
/// Used during streaming to detect the first action without waiting for the full response.
pub fn try_parse_first_action(buf: &str) -> Option<(ModelAction, usize)> {
    let brace_idx = buf.find('{')?;
    let candidate = &buf[brace_idx..];
    let mut de = Deserializer::from_str(candidate).into_iter::<serde_json::Value>();
    let value = de.next()?.ok()?;
    let consumed = de.byte_offset();
    let action = value_to_action(value)?;
    Some((action, brace_idx + consumed))
}

pub fn extract_first_json_object_span(s: &str) -> Option<(usize, usize)> {
    // Return byte offsets [start,end) for the first JSON object.
    // This is a best-effort extractor for logging. It handles nested braces and quoted strings.
    let bytes = s.as_bytes();
    let mut start: Option<usize> = None;
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;

    for (i, &b) in bytes.iter().enumerate() {
        let c = b as char;
        if start.is_none() {
            if c == '{' {
                start = Some(i);
                depth = 1;
            }
            continue;
        }

        if escape {
            escape = false;
            continue;
        }
        if in_string {
            if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }

        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((start.unwrap(), i + 1));
                }
            }
            _ => {}
        }
    }
    None
}

/// Return the trimmed text that appears before the first JSON object in the string.
/// If there is no JSON object at all, returns the full text (so it can be emitted
/// as a TextSegment for pure-text model responses).
/// Returns an empty string only if the input is empty or JSON starts at position 0.
pub fn text_before_first_json(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let Some((start, _end)) = extract_first_json_object_span(trimmed) else {
        // No JSON found — the entire response is plain text.
        return trimmed.to_string();
    };
    trimmed[..start].trim().to_string()
}

/// Heuristic: does plain text (no JSON) look like a substantive final answer
/// rather than the model "thinking out loud"?
///
/// Returns `true` when the text is long enough to be a real answer AND doesn't
/// start with phrases that signal the model intends to keep working.
pub fn looks_like_final_answer(raw: &str) -> bool {
    let trimmed = raw.trim();
    // Too short to be a real answer — likely thinking or a fragment.
    if trimmed.len() < 200 {
        return false;
    }
    // Starts with planning/thinking phrases → model wants to continue.
    let lower = trimmed.to_lowercase();
    let thinking_prefixes = [
        "i need to ",
        "i should ",
        "i'll ",
        "i will ",
        "let me ",
        "first, ",
        "next, ",
        "now i ",
        "to do this",
        "my plan is",
        "the approach",
        "step 1",
    ];
    for prefix in &thinking_prefixes {
        if lower.starts_with(prefix) {
            return false;
        }
    }
    true
}

fn truncate_text_chars(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let prefix: String = s.chars().take(max_chars).collect();
    format!("{prefix}…")
}

fn truncate_json_values(value: Value, max_chars: usize) -> Value {
    match value {
        Value::String(s) => Value::String(truncate_text_chars(&s, max_chars)),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|v| truncate_json_values(v, max_chars))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, truncate_json_values(v, max_chars)))
                .collect(),
        ),
        other => other,
    }
}

/// For logging/debugging only: split a model message into (text,json) parts with truncation.
///
/// - **text**: the non-JSON portion, truncated to `max_text_chars`
/// - **json**: the first JSON object (if any), with each string value truncated to `max_json_value_chars`
pub fn model_message_log_parts(
    raw: &str,
    max_text_chars: usize,
    max_json_value_chars: usize,
) -> (String, Option<Value>) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ("".to_string(), None);
    }

    let cleaned_trimmed = trimmed;
    let Some((start, end)) = extract_first_json_object_span(cleaned_trimmed) else {
        return (truncate_text_chars(cleaned_trimmed, max_text_chars), None);
    };

    let json_str = &cleaned_trimmed[start..end];
    let before = cleaned_trimmed[..start].trim();
    let after = cleaned_trimmed[end..].trim();
    let text = [before, after]
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    let json_value = serde_json::from_str::<Value>(json_str)
        .ok()
        .map(|v| truncate_json_values(v, max_json_value_chars));
    (truncate_text_chars(&text, max_text_chars), json_value)
}

#[cfg(test)]
mod tests {
    use super::{
        looks_like_final_answer, parse_all_actions, parse_first_action, text_before_first_json,
    };

    #[test]
    fn parse_first_action_preserves_generic_types_in_write_content() {
        let raw = r#"I'll apply the fix.
{"type":"tool","tool":"Write","args":{"path":"src/logging.rs","content":"static LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();\npub struct LoggingSettings<'a> { pub level: Option<&'a str>, pub retention_days: Option<u64>, }\n"}} "#;

        let action = parse_first_action(raw).expect("expected tool action");
        assert_eq!(action.tool, "Write");
        let content = action.args["content"]
            .as_str()
            .expect("content should be a string");
        assert!(content.contains("OnceLock<WorkerGuard>"));
        assert!(content.contains("Option<&'a str>"));
        assert!(content.contains("Option<u64>"));
    }

    #[test]
    fn parse_first_action_handles_wrapped_json_without_stripping() {
        let raw =
            "<search_indexing>\n{\"type\":\"tool\",\"tool\":\"Read\",\"args\":{\"path\":\"src/logging.rs\"}}\n</search_indexing>";
        let action = parse_first_action(raw).expect("expected wrapped tool action");
        assert_eq!(action.tool, "Read");
        assert_eq!(action.args["path"], "src/logging.rs");
    }

    #[test]
    fn parse_first_action_accepts_edit_tool() {
        let raw = r#"{"type":"tool","tool":"Edit","args":{"path":"src/logging.rs","old_string":"foo","new_string":"bar","replace_all":false}}"#;
        let action = parse_first_action(raw).expect("expected Edit tool action");
        assert_eq!(action.tool, "Edit");
        assert_eq!(action.args["path"], "src/logging.rs");
        assert_eq!(action.args["old_string"], "foo");
        assert_eq!(action.args["new_string"], "bar");
    }

    #[test]
    fn parse_all_actions_single_tool() {
        let raw = r#"{"type":"tool","tool":"Read","args":{"path":"src/main.rs"}}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].tool, "Read");
    }

    #[test]
    fn parse_all_actions_multiple_tools() {
        let raw = r#"I'll read two files.
{"type":"tool","tool":"Read","args":{"path":"src/main.rs"}}
Now let me also search:
{"type":"tool","tool":"Grep","args":{"query":"fn main","globs":["src/**"]}}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].tool, "Read");
        assert_eq!(actions[1].tool, "Grep");
    }

    #[test]
    fn parse_all_actions_finalize_task_is_ignored() {
        // finalize_task is no longer a valid action — only the Read tool should parse.
        let raw = r#"{"type":"tool","tool":"Read","args":{"path":"src/main.rs"}}
{"type":"finalize_task","packet":{"title":"test","user_stories":[],"acceptance_criteria":[],"mermaid_wireframe":null}}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].tool, "Read");
    }

    #[test]
    fn parse_all_actions_preserves_order() {
        let raw = r#"{"type":"tool","tool":"Glob","args":{"pattern":"*.rs"}}
{"type":"tool","tool":"Read","args":{"path":"a.rs"}}
{"type":"tool","tool":"Grep","args":{"query":"hello"}}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 3);
        let tools: Vec<&str> = actions.iter().map(|a| a.tool.as_str()).collect();
        assert_eq!(tools, vec!["Glob", "Read", "Grep"]);
    }

    #[test]
    fn parse_all_actions_name_field_as_tool() {
        let raw = r#"The user wants me to review logging.rs.{"name":"Read","args":{"path":"src/logging.rs"}}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].tool, "Read");
        assert_eq!(actions[0].args["path"], "src/logging.rs");
    }

    #[test]
    fn parse_all_actions_name_field_standalone() {
        let raw = r#"{"name":"Glob","args":{"globs":["**/*.rs"]}}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].tool, "Glob");
    }

    #[test]
    fn parse_enter_plan_mode_as_tool() {
        // enter_plan_mode is now parsed as a tool call to "EnterPlanMode".
        let raw = r#"{"type":"enter_plan_mode","reason":"complex refactoring needs research"}"#;
        let action = parse_first_action(raw).expect("expected EnterPlanMode tool action");
        assert_eq!(action.tool, "EnterPlanMode");
        assert_eq!(action.args["reason"], "complex refactoring needs research");
    }

    #[test]
    fn parse_all_actions_tool_then_enter_plan_mode() {
        let raw = r#"{"type":"tool","tool":"Read","args":{"path":"src/main.rs"}}
{"type":"enter_plan_mode","reason":"need to plan"}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].tool, "Read");
        assert_eq!(actions[1].tool, "EnterPlanMode");
        assert_eq!(actions[1].args["reason"], "need to plan");
    }

    #[test]
    fn parse_response_json_is_not_tool() {
        // {"response":"..."} is not a tool call — should fail to parse.
        let raw = r#"{"response":"Hi there! I can help you with that."}"#;
        assert!(parse_all_actions(raw).is_err());
    }

    #[test]
    fn parse_name_done_is_not_tool() {
        // {"name":"done"} is not a tool call — should be ignored.
        let raw = r#"{"name":"done","message":"Acked the casual greeting."}"#;
        assert!(parse_all_actions(raw).is_err());
    }

    #[test]
    fn parse_done_type_is_not_tool() {
        let raw = r#"{"type":"done","summary":"Code review completed."}"#;
        assert!(parse_all_actions(raw).is_err());
    }

    #[test]
    fn parse_task_with_flat_args() {
        let raw = r#"{"type":"Task","target_agent_id":"linggen-guide","task":"Introduce Linggen"}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].tool, "Task");
        assert_eq!(actions[0].args["target_agent_id"], "linggen-guide");
        assert_eq!(actions[0].args["task"], "Introduce Linggen");
    }

    #[test]
    fn parse_task_with_nested_args() {
        let raw = r#"{"type":"Task","args":{"target_agent_id":"coder","task":"Fix the bug"}}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].tool, "Task");
        assert_eq!(actions[0].args["target_agent_id"], "coder");
        assert_eq!(actions[0].args["task"], "Fix the bug");
    }

    #[test]
    fn parse_delegate_to_agent_backward_compat_flat() {
        let raw = r#"{"type":"delegate_to_agent","target_agent_id":"linggen-guide","task":"Introduce Linggen"}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].tool, "delegate_to_agent");
        assert_eq!(actions[0].args["target_agent_id"], "linggen-guide");
    }

    #[test]
    fn parse_delegate_to_agent_backward_compat_nested() {
        let raw = r#"{"type":"delegate_to_agent","args":{"target_agent_id":"coder","task":"Fix the bug"}}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].tool, "delegate_to_agent");
        assert_eq!(actions[0].args["target_agent_id"], "coder");
    }

    #[test]
    fn parse_update_plan_as_tool() {
        let raw = r#"I need to create a plan.
{"type":"update_plan","items":[{"id":"1","title":"Step one","status":"completed"},{"id":"2","title":"Step two","status":"in_progress"},{"id":"3","title":"Step three","status":"pending"}]}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].tool, "UpdatePlan");
        let items = actions[0].args["items"].as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["title"], "Step one");
    }

    #[test]
    fn text_before_first_json_extracts_prose() {
        let raw = r#"Let me read the file.
{"type":"tool","tool":"Read","args":{"path":"src/main.rs"}}"#;
        assert_eq!(text_before_first_json(raw), "Let me read the file.");
    }

    #[test]
    fn text_before_first_json_returns_full_text_when_no_json() {
        assert_eq!(text_before_first_json("No JSON here"), "No JSON here");
    }

    #[test]
    fn text_before_first_json_empty_when_json_at_start() {
        let raw = r#"{"type":"tool","tool":"Read","args":{"path":"a.rs"}}"#;
        assert_eq!(text_before_first_json(raw), "");
    }

    #[test]
    fn text_before_first_json_trims_whitespace() {
        let raw = r#"  I'll fix this.  {"type":"done","message":"ok"}"#;
        assert_eq!(text_before_first_json(raw), "I'll fix this.");
    }

    #[test]
    fn looks_like_final_answer_short_text_is_not_final() {
        assert!(!looks_like_final_answer("I need to read the file first."));
        assert!(!looks_like_final_answer("Let me check that."));
        assert!(!looks_like_final_answer(""));
    }

    #[test]
    fn looks_like_final_answer_thinking_prefix_is_not_final() {
        let long_thinking = format!("I need to {}", "x".repeat(300));
        assert!(!looks_like_final_answer(&long_thinking));
        let long_planning = format!("Let me {}", "x".repeat(300));
        assert!(!looks_like_final_answer(&long_planning));
    }

    #[test]
    fn looks_like_final_answer_long_substantive_text_is_final() {
        let review = format!(
            "## Code Review\n\nThe logging module is well-structured. {}",
            "Details here. ".repeat(30)
        );
        assert!(looks_like_final_answer(&review));
    }

    #[test]
    fn parse_done_bool_is_not_tool() {
        let raw = r#"{"done":true,"summary":"Reviewed the codebase and found issues."}"#;
        assert!(parse_all_actions(raw).is_err());
    }
}

use crate::engine::TaskPacket;
use anyhow::Result;
use serde::Deserialize;
use serde_json::de::Deserializer;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ModelAction {
    #[serde(rename = "tool")]
    Tool {
        tool: String,
        args: serde_json::Value,
    },
    #[serde(rename = "patch")]
    Patch { diff: String },
    #[serde(rename = "finalize_task")]
    FinalizeTask { packet: TaskPacket },
    #[serde(rename = "done")]
    Done {
        #[serde(default)]
        message: Option<String>,
    },
    #[serde(rename = "enter_plan_mode")]
    EnterPlanMode {
        #[serde(default)]
        reason: Option<String>,
    },
    #[serde(rename = "update_plan")]
    UpdatePlan {
        #[serde(default)]
        items: Vec<serde_json::Value>,
    },
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
    if let Ok(action) = serde_json::from_value::<ModelAction>(value.clone()) {
        return Some(action);
    }

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
        // Intercept "done"/"Done" — models sometimes emit {"name":"done","message":"..."}
        // which should be a Done action, not a tool call.
        if tool.eq_ignore_ascii_case("done") {
            let msg = obj
                .get("message")
                .or_else(|| obj.get("text"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            return Some(ModelAction::Done { message: msg });
        }
        if !is_supported_model_tool(tool) {
            return None;
        }
        return Some(ModelAction::Tool {
            tool: tool.to_string(),
            args,
        });
    }

    let tool_name = match action_type {
        "Read" | "Grep" | "Write" | "Edit" | "Glob" | "Bash" | "capture_screenshot"
        | "lock_paths" | "unlock_paths" | "Task" | "delegate_to_agent" => action_type,
        // Known non-tool action types that should not be treated as tool shorthands.
        "" | "patch" | "finalize_task" | "done" | "enter_plan_mode" => {
            // Fallback: if the object has a text-like field but no action type,
            // treat it as a Done message. This handles models that emit
            // {"response":"..."} or {"message":"..."} instead of a proper action.
            if action_type.is_empty() {
                for key in &["response", "message", "text", "content", "answer"] {
                    if let Some(val) = obj.get(*key).and_then(|v| v.as_str()) {
                        return Some(ModelAction::Done {
                            message: Some(val.to_string()),
                        });
                    }
                }
            }
            return None;
        }
        // Unknown type with args present: treat as a skill tool shorthand
        // (e.g. {"type":"my_skill_tool","args":{...}}).
        other if obj.contains_key("args") || obj.contains_key("tool_args") => other,
        _ => return None,
    };

    Some(ModelAction::Tool {
        tool: tool_name.to_string(),
        args,
    })
}

fn is_supported_model_tool(_tool: &str) -> bool {
    // Always accept: the ToolRegistry rejects unknown tools at execution time
    // with a proper error message, which is better than silently dropping the action.
    true
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
    use super::{parse_all_actions, parse_first_action, text_before_first_json, ModelAction};

    #[test]
    fn parse_first_action_preserves_generic_types_in_write_content() {
        let raw = r#"I'll apply the fix.
{"type":"tool","tool":"Write","args":{"path":"src/logging.rs","content":"static LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();\npub struct LoggingSettings<'a> { pub level: Option<&'a str>, pub retention_days: Option<u64>, }\n"}} "#;

        let action = parse_first_action(raw).expect("expected tool action");
        match action {
            ModelAction::Tool { tool, args } => {
                assert_eq!(tool, "Write");
                let content = args["content"].as_str().expect("content should be a string");
                assert!(content.contains("OnceLock<WorkerGuard>"));
                assert!(content.contains("Option<&'a str>"));
                assert!(content.contains("Option<u64>"));
            }
            _ => panic!("expected tool action"),
        }
    }

    #[test]
    fn parse_first_action_handles_wrapped_json_without_stripping() {
        let raw =
            "<search_indexing>\n{\"type\":\"tool\",\"tool\":\"Read\",\"args\":{\"path\":\"src/logging.rs\"}}\n</search_indexing>";
        let action = parse_first_action(raw).expect("expected wrapped tool action");
        match action {
            ModelAction::Tool { tool, args } => {
                assert_eq!(tool, "Read");
                assert_eq!(args["path"], "src/logging.rs");
            }
            _ => panic!("expected tool action"),
        }
    }

    #[test]
    fn parse_first_action_accepts_edit_tool() {
        let raw = r#"{"type":"tool","tool":"Edit","args":{"path":"src/logging.rs","old_string":"foo","new_string":"bar","replace_all":false}}"#;
        let action = parse_first_action(raw).expect("expected Edit tool action");
        match action {
            ModelAction::Tool { tool, args } => {
                assert_eq!(tool, "Edit");
                assert_eq!(args["path"], "src/logging.rs");
                assert_eq!(args["old_string"], "foo");
                assert_eq!(args["new_string"], "bar");
            }
            _ => panic!("expected tool action"),
        }
    }

    #[test]
    fn parse_all_actions_single_tool() {
        let raw = r#"{"type":"tool","tool":"Read","args":{"path":"src/main.rs"}}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            ModelAction::Tool { tool, .. } => assert_eq!(tool, "Read"),
            _ => panic!("expected tool action"),
        }
    }

    #[test]
    fn parse_all_actions_multiple_tools() {
        let raw = r#"I'll read two files.
{"type":"tool","tool":"Read","args":{"path":"src/main.rs"}}
Now let me also search:
{"type":"tool","tool":"Grep","args":{"query":"fn main","globs":["src/**"]}}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 2);
        match &actions[0] {
            ModelAction::Tool { tool, .. } => assert_eq!(tool, "Read"),
            _ => panic!("expected Read tool"),
        }
        match &actions[1] {
            ModelAction::Tool { tool, .. } => assert_eq!(tool, "Grep"),
            _ => panic!("expected Grep tool"),
        }
    }

    #[test]
    fn parse_all_actions_tool_then_finalize() {
        let raw = r#"{"type":"tool","tool":"Read","args":{"path":"src/main.rs"}}
{"type":"finalize_task","packet":{"title":"test","user_stories":[],"acceptance_criteria":[],"mermaid_wireframe":null}}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 2);
        match &actions[0] {
            ModelAction::Tool { tool, .. } => assert_eq!(tool, "Read"),
            _ => panic!("expected tool action"),
        }
        match &actions[1] {
            ModelAction::FinalizeTask { packet } => assert_eq!(packet.title, "test"),
            _ => panic!("expected finalize action"),
        }
    }

    #[test]
    fn parse_all_actions_preserves_order() {
        let raw = r#"{"type":"tool","tool":"Glob","args":{"pattern":"*.rs"}}
{"type":"tool","tool":"Read","args":{"path":"a.rs"}}
{"type":"tool","tool":"Grep","args":{"query":"hello"}}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 3);
        let tools: Vec<&str> = actions
            .iter()
            .filter_map(|a| match a {
                ModelAction::Tool { tool, .. } => Some(tool.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(tools, vec!["Glob", "Read", "Grep"]);
    }

    #[test]
    fn parse_all_actions_name_field_as_tool() {
        // Models sometimes emit {"name":"Read","args":{...}} instead of
        // {"type":"tool","tool":"Read","args":{...}}.
        let raw = r#"The user wants me to review logging.rs.{"name":"Read","args":{"path":"src/logging.rs"}}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            ModelAction::Tool { tool, args } => {
                assert_eq!(tool, "Read");
                assert_eq!(args["path"], "src/logging.rs");
            }
            _ => panic!("expected tool action"),
        }
    }

    #[test]
    fn parse_all_actions_name_field_standalone() {
        let raw = r#"{"name":"Glob","args":{"globs":["**/*.rs"]}}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            ModelAction::Tool { tool, .. } => assert_eq!(tool, "Glob"),
            _ => panic!("expected tool action"),
        }
    }

    #[test]
    fn parse_enter_plan_mode_without_reason() {
        let raw = r#"{"type":"enter_plan_mode"}"#;
        let action = parse_first_action(raw).expect("expected enter_plan_mode action");
        match action {
            ModelAction::EnterPlanMode { reason } => {
                assert!(reason.is_none());
            }
            _ => panic!("expected EnterPlanMode, got {:?}", action),
        }
    }

    #[test]
    fn parse_enter_plan_mode_with_reason() {
        let raw = r#"{"type":"enter_plan_mode","reason":"complex refactoring needs research"}"#;
        let action = parse_first_action(raw).expect("expected enter_plan_mode action");
        match action {
            ModelAction::EnterPlanMode { reason } => {
                assert_eq!(reason.as_deref(), Some("complex refactoring needs research"));
            }
            _ => panic!("expected EnterPlanMode, got {:?}", action),
        }
    }

    #[test]
    fn parse_enter_plan_mode_not_treated_as_tool() {
        // enter_plan_mode should be parsed as EnterPlanMode, not as a Tool action.
        let raw = r#"{"type":"enter_plan_mode","reason":"planning needed"}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        assert!(
            matches!(&actions[0], ModelAction::EnterPlanMode { .. }),
            "expected EnterPlanMode, got {:?}",
            actions[0]
        );
    }

    #[test]
    fn parse_all_actions_tool_then_enter_plan_mode() {
        let raw = r#"{"type":"tool","tool":"Read","args":{"path":"src/main.rs"}}
{"type":"enter_plan_mode","reason":"need to plan"}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 2);
        match &actions[0] {
            ModelAction::Tool { tool, .. } => assert_eq!(tool, "Read"),
            _ => panic!("expected tool action"),
        }
        match &actions[1] {
            ModelAction::EnterPlanMode { reason } => {
                assert_eq!(reason.as_deref(), Some("need to plan"));
            }
            _ => panic!("expected EnterPlanMode"),
        }
    }

    #[test]
    fn parse_response_json_as_done() {
        // Models sometimes emit {"response":"..."} instead of {"type":"done","message":"..."}.
        // This should be treated as a Done action.
        let raw = r#"{"response":"Hi there! I can help you with that."}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            ModelAction::Done { message } => {
                assert_eq!(
                    message.as_deref(),
                    Some("Hi there! I can help you with that.")
                );
            }
            _ => panic!("expected Done action, got {:?}", actions[0]),
        }
    }

    #[test]
    fn parse_name_done_as_done_action() {
        // Models sometimes emit {"name":"done","message":"..."} which should be
        // parsed as ModelAction::Done, not as a Tool call to "done".
        let raw = r#"{"name":"done","message":"Acked the casual greeting."}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], ModelAction::Done { message } if message.as_deref() == Some("Acked the casual greeting.")));
    }

    #[test]
    fn parse_name_done_capitalized_as_done_action() {
        let raw = r#"{"name":"Done","message":"All finished."}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], ModelAction::Done { message } if message.as_deref() == Some("All finished.")));
    }

    #[test]
    fn parse_message_json_as_done() {
        let raw = r#"{"message":"Task complete."}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], ModelAction::Done { message } if message.as_deref() == Some("Task complete.")));
    }

    #[test]
    fn parse_text_json_as_done() {
        let raw = r#"{"text":"Here is the answer."}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], ModelAction::Done { message } if message.as_deref() == Some("Here is the answer.")));
    }

    #[test]
    fn parse_task_with_flat_args() {
        // Models emit Task with args at top level instead of nested under "args".
        let raw = r#"{"type":"Task","target_agent_id":"linggen-guide","task":"Introduce Linggen"}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            ModelAction::Tool { tool, args } => {
                assert_eq!(tool, "Task");
                assert_eq!(args["target_agent_id"], "linggen-guide");
                assert_eq!(args["task"], "Introduce Linggen");
            }
            _ => panic!("expected tool action, got {:?}", actions[0]),
        }
    }

    #[test]
    fn parse_task_with_nested_args() {
        // Proper format with args nested should still work.
        let raw = r#"{"type":"Task","args":{"target_agent_id":"coder","task":"Fix the bug"}}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            ModelAction::Tool { tool, args } => {
                assert_eq!(tool, "Task");
                assert_eq!(args["target_agent_id"], "coder");
                assert_eq!(args["task"], "Fix the bug");
            }
            _ => panic!("expected tool action, got {:?}", actions[0]),
        }
    }

    #[test]
    fn parse_delegate_to_agent_backward_compat_flat() {
        // Backward compat: old "delegate_to_agent" format still parses.
        let raw = r#"{"type":"delegate_to_agent","target_agent_id":"linggen-guide","task":"Introduce Linggen"}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            ModelAction::Tool { tool, args } => {
                assert_eq!(tool, "delegate_to_agent");
                assert_eq!(args["target_agent_id"], "linggen-guide");
                assert_eq!(args["task"], "Introduce Linggen");
            }
            _ => panic!("expected tool action, got {:?}", actions[0]),
        }
    }

    #[test]
    fn parse_delegate_to_agent_backward_compat_nested() {
        // Backward compat: old "delegate_to_agent" format with nested args still parses.
        let raw = r#"{"type":"delegate_to_agent","args":{"target_agent_id":"coder","task":"Fix the bug"}}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            ModelAction::Tool { tool, args } => {
                assert_eq!(tool, "delegate_to_agent");
                assert_eq!(args["target_agent_id"], "coder");
                assert_eq!(args["task"], "Fix the bug");
            }
            _ => panic!("expected tool action, got {:?}", actions[0]),
        }
    }

    #[test]
    fn parse_update_plan_action() {
        let raw = r#"I need to create a plan.
{"type":"update_plan","items":[{"id":"1","title":"Step one","status":"completed"},{"id":"2","title":"Step two","status":"in_progress"},{"id":"3","title":"Step three","status":"pending"}]}"#;
        let actions = parse_all_actions(raw).unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            ModelAction::UpdatePlan { items } => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0]["title"], "Step one");
                assert_eq!(items[1]["status"], "in_progress");
            }
            _ => panic!("expected UpdatePlan, got {:?}", actions[0]),
        }
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
}

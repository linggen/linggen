use crate::engine::tools::ToolResult;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;

pub fn render_tool_result(r: &ToolResult) -> String {
    match r {
        ToolResult::FileList(v) => {
            if v.is_empty() {
                "files:\n(no files)".to_string()
            } else {
                format!("files:\n{}", v.join("\n"))
            }
        }
        ToolResult::FileContent {
            path,
            content,
            truncated,
        } => {
            format!("Read: {} (truncated: {})\n{}", path, truncated, content)
        }
        ToolResult::SearchMatches(v) => {
            let mut out = String::new();
            out.push_str("Grep matches:\n");
            if v.is_empty() {
                out.push_str("(no matches)\n");
            } else {
                for m in v {
                    out.push_str(&format!("{}:{}:{}\n", m.path, m.line, m.snippet));
                }
            }
            out
        }
        ToolResult::CommandOutput {
            exit_code,
            stdout,
            stderr,
        } => {
            format!(
                "Bash output (exit_code: {:?}):\nSTDOUT:\n{}\nSTDERR:\n{}",
                exit_code, stdout, stderr
            )
        }
        ToolResult::Screenshot { url, base64 } => {
            format!(
                "screenshot_captured: {} (base64 length: {})",
                url,
                base64.len()
            )
        }
        ToolResult::Success(msg) => format!("success: {}", msg),
        ToolResult::LockResult { acquired, denied } => {
            format!("lock_result: acquired={:?}, denied={:?}", acquired, denied)
        }
        ToolResult::AgentOutcome(outcome) => match outcome {
            crate::engine::AgentOutcome::None => "agent completed (no structured result)".to_string(),
            crate::engine::AgentOutcome::Task(t) => format!("agent produced task: {}", t.title),
            crate::engine::AgentOutcome::Patch(diff) => format!("agent produced patch ({} bytes)", diff.len()),
            crate::engine::AgentOutcome::Plan(p) => format!("agent produced plan: {}, status={:?}", p.summary, p.status),
            crate::engine::AgentOutcome::PlanModeRequested { reason } => format!("agent requested plan mode: {}", reason.as_deref().unwrap_or("(no reason)")),
        }
        ToolResult::WebSearchResults { query, results } => {
            let mut out = format!("WebSearch: \"{}\" ({} results)\n", query, results.len());
            for (i, r) in results.iter().enumerate() {
                out.push_str(&format!(
                    "{}. {} — {}\n   {}\n",
                    i + 1,
                    r.title,
                    r.url,
                    r.snippet
                ));
            }
            out
        }
        ToolResult::WebFetchContent {
            url,
            content,
            content_type,
            truncated,
        } => {
            format!(
                "WebFetch: {} (type: {}, truncated: {})\n{}",
                url, content_type, truncated, content
            )
        }
        ToolResult::AskUserResponse { answers } => {
            let mut out = String::from("User responded:\n");
            for a in answers {
                let selected = a.selected.join(", ");
                if let Some(ref custom) = a.custom_text {
                    out.push_str(&format!("  Q{}: custom: \"{}\"\n", a.question_index, custom));
                } else {
                    out.push_str(&format!("  Q{}: {}\n", a.question_index, selected));
                }
            }
            out
        }
    }
}

fn preview_text(content: &str, max_lines: usize, max_chars: usize) -> (String, bool) {
    let mut out = String::new();
    let mut lines = 0usize;
    let mut truncated = false;

    for line in content.lines() {
        if lines >= max_lines {
            truncated = true;
            break;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
        lines += 1;

        if out.len() >= max_chars {
            truncated = true;
            out.truncate(max_chars);
            break;
        }
    }

    (out, truncated)
}

/// Render tool results for session files / UI without dumping large payloads (e.g. full files).
pub fn render_tool_result_public(r: &ToolResult) -> String {
    match r {
        ToolResult::FileContent {
            path,
            truncated,
            ..
        } => {
            format!(
                "Read: {} (truncated: {})\n(content omitted in chat; open the file viewer for full text)",
                path, truncated
            )
        }
        ToolResult::WebFetchContent {
            url,
            content,
            content_type,
            truncated,
        } => {
            let (preview, preview_truncated) = preview_text(content, 30, 2000);
            let shown_note = if preview_truncated { " (preview)" } else { "" };
            format!(
                "WebFetch: {} (type: {}, truncated: {}){}\n{}",
                url, content_type, truncated, shown_note, preview
            )
        }
        ToolResult::WebSearchResults { .. } => render_tool_result(r),
        ToolResult::AskUserResponse { .. } => render_tool_result(r),
        other => render_tool_result(other),
    }
}

pub fn normalize_tool_path_arg(ws_root: &Path, args: &serde_json::Value) -> Option<String> {
    use std::path::Component;

    let raw = args
        .get("path")
        .or_else(|| args.get("file"))
        .or_else(|| args.get("filepath"))
        .and_then(|v| v.as_str())?;
    let raw_path = Path::new(raw);
    let rel = if raw_path.is_absolute() {
        raw_path.strip_prefix(ws_root).ok()?.to_path_buf()
    } else {
        raw_path.to_path_buf()
    };
    if rel.as_os_str().is_empty() {
        return None;
    }
    if rel.components().any(|c| matches!(c, Component::ParentDir)) {
        return None;
    }
    Some(rel.to_string_lossy().to_string())
}

pub fn sanitize_tool_args_for_display(tool: &str, args: &serde_json::Value) -> serde_json::Value {
    let mut safe = args.clone();
    if let Some(obj) = safe.as_object_mut() {
        if matches!(tool, "Write") {
            if let Some(content) = obj.get("content").and_then(|v| v.as_str()) {
                let bytes = content.len();
                let lines = content.lines().count();
                obj.insert(
                    "content".to_string(),
                    serde_json::json!(format!("<omitted:{} bytes, {} lines>", bytes, lines)),
                );
            }
        } else if matches!(tool, "Edit") {
            for key in ["old_string", "new_string", "old", "new", "old_text", "new_text", "oldText", "newText", "search", "replace", "from", "to"] {
                if let Some(content) = obj.get(key).and_then(|v| v.as_str()) {
                    let bytes = content.len();
                    let lines = content.lines().count();
                    obj.insert(
                        key.to_string(),
                        serde_json::json!(format!("<omitted:{} bytes, {} lines>", bytes, lines)),
                    );
                }
            }
        }
    }
    safe
}

pub fn tool_call_signature(tool: &str, args: &serde_json::Value) -> String {
    if matches!(tool, "Write") {
        let path = args
            .get("path")
            .or_else(|| args.get("file"))
            .or_else(|| args.get("filepath"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        return format!(
            "{}|path={}|content_hash={:x}|len={}",
            tool,
            path,
            hasher.finish(),
            content.len()
        );
    }
    if matches!(tool, "Edit") {
        let path = args
            .get("path")
            .or_else(|| args.get("file"))
            .or_else(|| args.get("filepath"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let old_string = args
            .get("old_string")
            .or_else(|| args.get("old"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let new_string = args
            .get("new_string")
            .or_else(|| args.get("new"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let replace_all = args
            .get("replace_all")
            .or_else(|| args.get("all"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let mut old_hasher = DefaultHasher::new();
        old_string.hash(&mut old_hasher);
        let mut new_hasher = DefaultHasher::new();
        new_string.hash(&mut new_hasher);
        return format!(
            "{}|path={}|old_hash={:x}|new_hash={:x}|old_len={}|new_len={}|all={}",
            tool,
            path,
            old_hasher.finish(),
            new_hasher.finish(),
            old_string.len(),
            new_string.len(),
            replace_all
        );
    }
    format!("{}|{}", tool, args)
}

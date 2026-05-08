//! Permission prompt construction and answer parsing.
//!
//! These build/parse the AskUser widget shown to the user when a tool call
//! exceeds the session ceiling. Pure functions — no I/O, no state.

use crate::engine::render::normalize_tool_path_arg;
use crate::engine::tools::{AskUserOption, AskUserQuestion};
use std::path::Path;

use super::model::{PermissionAction, PermissionMode};

pub fn permission_target_summary(tool: &str, args: &serde_json::Value, cwd: &Path) -> String {
    match tool {
        "Write" | "Edit" => normalize_tool_path_arg(cwd, args)
            .unwrap_or_else(|| "<unknown file>".to_string()),
        "Bash" => args
            .get("cmd")
            .or_else(|| args.get("command"))
            .and_then(|v| v.as_str())
            .map(|cmd| {
                if cmd.len() > 80 {
                    format!("{}...", &cmd[..77])
                } else {
                    cmd.to_string()
                }
            })
            .unwrap_or_else(|| "<unknown command>".to_string()),
        "Patch" => args
            .get("diff")
            .or_else(|| args.get("patch"))
            .and_then(|v| v.as_str())
            .and_then(|diff| {
                diff.lines().find_map(|line| {
                    line.strip_prefix("+++ b/")
                        .or_else(|| line.strip_prefix("+++ "))
                        .map(|s| s.to_string())
                })
            })
            .unwrap_or_else(|| "<patch>".to_string()),
        "WebFetch" => args
            .get("url")
            .and_then(|v| v.as_str())
            .map(|url| truncate_for_prompt(url, 120))
            .unwrap_or_else(|| "<unknown URL>".to_string()),
        "WebSearch" => args
            .get("query")
            .and_then(|v| v.as_str())
            .map(|q| truncate_for_prompt(q, 120))
            .unwrap_or_else(|| "<unknown query>".to_string()),
        _ => skill_tool_summary(args).unwrap_or_else(|| tool.to_string()),
    }
}

fn skill_tool_summary(args: &serde_json::Value) -> Option<String> {
    for key in &["query", "content", "id", "endpoint", "path", "url"] {
        if let Some(v) = args.get(*key).and_then(|v| v.as_str()) {
            if !v.is_empty() {
                return Some(truncate_for_prompt(v, 120));
            }
        }
    }
    None
}

fn truncate_for_prompt(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}...", &s[..max.saturating_sub(3)])
    } else {
        s.to_string()
    }
}

/// Build AskUser question for an ExceedsCeiling prompt.
pub fn build_exceeds_ceiling_question(
    tool_summary: &str,
    target_mode: &PermissionMode,
    path: &str,
) -> AskUserQuestion {
    AskUserQuestion {
        question: tool_summary.to_string(),
        header: "Permission".to_string(),
        options: vec![
            AskUserOption {
                label: format!("Switch this folder to {}", target_mode),
                description: Some(format!("Grants {} on {} and children", target_mode, path)),
                preview: None,
            },
            AskUserOption {
                label: "Allow once".to_string(),
                description: Some("One-time approval, no persistence".to_string()),
                preview: None,
            },
            AskUserOption {
                label: "Deny".to_string(),
                description: Some("Block this action".to_string()),
                preview: None,
            },
        ],
        multi_select: false,
    }
}

/// Parse user response to an ExceedsCeiling prompt.
pub fn parse_exceeds_ceiling_answer(
    selected: &str,
    target_mode: &PermissionMode,
) -> PermissionAction {
    if selected == "Allow once" {
        PermissionAction::AllowOnce
    } else if selected == format!("Switch this folder to {}", target_mode) {
        PermissionAction::AllowSession
    } else {
        PermissionAction::Deny
    }
}

//! Permission prompt construction and answer parsing.
//!
//! These build/parse the AskUser widget shown to the user when a tool call
//! exceeds the session ceiling. Pure functions — no I/O, no state.

use crate::engine::render::normalize_tool_path_arg;
use crate::engine::tools::{
    AskUserAnswer, AskUserOption, AskUserQuestion, DirectoryPermissionMetadata,
};
use std::path::Path;

use super::model::PermissionAction;
use super::model::PermissionMode;

pub fn permission_target_summary(tool: &str, args: &serde_json::Value, cwd: &Path) -> String {
    match tool {
        "Write" | "Edit" => {
            normalize_tool_path_arg(cwd, args).unwrap_or_else(|| "<unknown file>".to_string())
        }
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
        "WebSearch" | "ImageSearch" | "VideoSearch" => args
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
    directories: &[String],
) -> AskUserQuestion {
    AskUserQuestion {
        question: tool_summary.to_string(),
        header: "Permission".to_string(),
        permission: Some(DirectoryPermissionMetadata {
            path: path.to_string(),
            mode: *target_mode,
        }),
        options: {
            // One button per directory, broadest first, so the repo root is a
            // single click. Then the same level everywhere, admin everywhere
            // when this ask is below admin, and Deny.
            let mut options = Vec::new();
            for dir in directories {
                options.push(AskUserOption {
                    label: format!("grant:at:{target_mode}:{dir}"),
                    description: Some(format!(
                        "{dir} and its children — {target_mode} for this session"
                    )),
                    preview: None,
                });
            }
            options.push(AskUserOption {
                label: format!("grant:all:{target_mode}"),
                description: Some(format!(
                    "All directories; replaces existing directory grants — {target_mode} for this session"
                )),
                preview: None,
            });
            if *target_mode != PermissionMode::Admin {
                options.push(AskUserOption {
                    label: "grant:all:admin".into(),
                    description: Some(
                        "All directories; replaces existing directory grants — admin for this session"
                            .into(),
                    ),
                    preview: None,
                });
            }
            options.push(AskUserOption {
                label: "Deny".into(),
                description: Some("Block this action".into()),
                preview: None,
            });
            options
        },
        multi_select: false,
    }
}

/// A native permission answer must be exactly one offered option for question zero.
/// A directory answer grants that offered path at the needed level. Anything
/// that was not offered denies, so a client cannot name its own path.
pub fn parse_exceeds_ceiling_answer(
    answers: &[AskUserAnswer],
    target_mode: &PermissionMode,
    offered: &[String],
) -> PermissionAction {
    if answers.len() != 1 || answers[0].question_index != 0 {
        return PermissionAction::Deny;
    }
    let answer = &answers[0];
    if answer.custom_text.as_deref().is_some_and(|s| !s.is_empty()) {
        return PermissionAction::Deny;
    }
    if answer.selected.len() != 1 {
        return PermissionAction::Deny;
    }
    let selected = answer.selected[0].as_str();
    if !offered.iter().any(|label| label == selected) {
        return PermissionAction::Deny;
    }
    if let Some(dir) = selected
        .strip_prefix(&format!("grant:at:{target_mode}:"))
        .filter(|dir| !dir.is_empty())
    {
        return PermissionAction::AllowDirectory {
            all: false,
            mode: *target_mode,
            path: dir.to_string(),
        };
    }
    if selected == format!("grant:all:{target_mode}") {
        return PermissionAction::AllowDirectory {
            all: true,
            mode: *target_mode,
            path: "*".to_string(),
        };
    }
    if selected == "grant:all:admin" {
        return PermissionAction::AllowDirectory {
            all: true,
            mode: PermissionMode::Admin,
            path: "*".to_string(),
        };
    }
    PermissionAction::Deny
}

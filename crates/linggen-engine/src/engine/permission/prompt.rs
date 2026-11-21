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
            .map(|cmd| truncate_for_prompt(cmd, 80))
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

/// `s` cut to `max` bytes with `...`, never inside a character (a byte
/// cut through a CJK character panics).
fn truncate_for_prompt(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max.saturating_sub(3);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
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
            path: super::model::display_path(std::path::Path::new(path)),
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
                        "{} and its children — {target_mode} for this session",
                        super::model::display_path(std::path::Path::new(dir))
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
                label: DENY.into(),
                description: Some("Block this action".into()),
                preview: None,
            });
            options
        },
        multi_select: false,
    }
}

/// The option a caller's "run this one action" answer selects.
pub const ALLOW_ONCE: &str = "Allow once";

/// The option that blocks the action.
pub const DENY: &str = "Deny";

/// The Chinese for the two permission answers the engine writes — the words
/// the page shows for them (`ui/src/lib/strings-zh.ts`, the page's one table;
/// its two entries must read the same — `the_page_shows_what_the_table_says`).
/// A spoken answer may be said in either form
/// (`tools::builtin::answer_prompt::relay_heard`), whatever language the page
/// is in: a person who speaks Chinese says 拒绝 to an English page too, and
/// the words mean one thing. Nothing else is here: a label that is not one of
/// these two is only ever said as it is written.
const SAID_IN_CHINESE: [(&str, &str); 2] = [(ALLOW_ONCE, "允许一次"), (DENY, "拒绝")];

/// The Chinese form of `label`; `None` when it has none.
pub fn label_in_chinese(label: &str) -> Option<&'static str> {
    SAID_IN_CHINESE.iter().find(|(english, _)| *english == label).map(|(_, chinese)| *chinese)
}

/// The option label that grants `path` at `mode` for the session.
pub fn grant_at_label(mode: &PermissionMode, path: &str) -> String {
    format!("grant:at:{mode}:{path}")
}

/// The question a non-interactive session raises to its caller: the target
/// itself, its folders, once, or deny — never a session-wide grant.
pub fn build_caller_question(
    tool_summary: &str,
    target_mode: &PermissionMode,
    path: &str,
    directories: &[String],
) -> AskUserQuestion {
    let grant = |dir: &str, what: &str| AskUserOption {
        label: grant_at_label(target_mode, dir),
        description: Some(format!("{what} — {target_mode} for this session")),
        preview: None,
    };
    let mut options: Vec<AskUserOption> = directories
        .iter()
        .map(|dir| grant(dir, &format!("{dir} and its children")))
        .collect();
    if !directories.iter().any(|dir| dir == path) {
        options.push(grant(path, path));
    }
    options.push(AskUserOption {
        label: ALLOW_ONCE.into(),
        description: Some("Run this one action".into()),
        preview: None,
    });
    options.push(AskUserOption {
        label: DENY.into(),
        description: Some("Block this action".into()),
        preview: None,
    });
    AskUserQuestion {
        question: tool_summary.to_string(),
        header: "Permission".to_string(),
        permission: Some(DirectoryPermissionMetadata {
            path: super::model::display_path(std::path::Path::new(path)),
            mode: *target_mode,
        }),
        options,
        multi_select: false,
    }
}

/// The question for an outward command (`PromptKind::ExceedsCeiling`
/// with `outward`), to the person or the caller alike: this one command,
/// once, or deny — no grant covers the next one.
pub fn build_outward_question(
    tool_summary: &str,
    target_mode: &PermissionMode,
    path: &str,
) -> AskUserQuestion {
    let option = |label: &str, description: &str| AskUserOption {
        label: label.into(),
        description: Some(description.into()),
        preview: None,
    };
    AskUserQuestion {
        question: tool_summary.to_string(),
        header: "Permission".to_string(),
        permission: Some(DirectoryPermissionMetadata {
            path: super::model::display_path(std::path::Path::new(path)),
            mode: *target_mode,
        }),
        options: vec![
            option(ALLOW_ONCE, "Run this one command; it leaves this machine"),
            option(DENY, "Block this action"),
        ],
        multi_select: false,
    }
}

/// An action a rule in the person's Claude Code settings asks before, or a
/// change to one of those files (`claude_rules`): allow once or deny, like [`build_outward_question`] —
/// no grant can satisfy the rule.
pub fn build_rule_question(
    tool_summary: &str,
    target_mode: &PermissionMode,
    path: &str,
) -> AskUserQuestion {
    let mut question = build_outward_question(tool_summary, target_mode, path);
    question.options[0].description = Some("Run this one action; it is asked every time".into());
    question
}

/// An action that destroys what it acts on, asked of the person or the
/// caller: allow once or deny, like [`build_outward_question`] — no grant
/// stands for the next one.
pub fn build_destroy_question(
    tool_summary: &str,
    target_mode: &PermissionMode,
    path: &str,
) -> AskUserQuestion {
    let mut question = build_outward_question(tool_summary, target_mode, path);
    question.options[0].description = Some("Run this one action; it cannot be undone".into());
    question
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
    if selected == ALLOW_ONCE {
        return PermissionAction::AllowOnce;
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

//! The door's `app` tool: an agent outside Linggen uses what the person's
//! apps do. The list is the one a session's `AppTool` lists and every run
//! is judged by `skill::judge` as the action it is (`skill::outside`); this
//! side finds the apps and the owner's setting, and words the answer.
//!
//! For the owner at this Mac alone: anyone else is refused before an app
//! is looked up, whatever the app is called or whether it exists.

use super::catalog::AppOp;
use super::{tool_content, McpDeps};
use crate::engine::skill::outside::{host_tools, open_to_host, run_for_host};
use crate::engine::skill::tools::{action_row, app_line, AppLine};
use crate::engine::skill::Skill;
use crate::engine::tools::ToolResult;
use serde_json::Value;
use std::sync::Arc;

/// What a caller that is not the owner at this Mac is told.
const ONLY_AT_THIS_MAC: &str = "the app tool answers only the owner, at this Mac";

pub(super) async fn run(deps: &McpDeps<'_>, op: AppOp, args: Value) -> Value {
    if !deps.at_this_mac {
        return tool_content(ONLY_AT_THIS_MAC.to_string(), true);
    }
    let Some(state) = deps.state else {
        return tool_content(
            "the app tool is unavailable in this context (no daemon state)".to_string(),
            true,
        );
    };
    let field = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
    };
    let opened = state.manager.get_config_snapshot().await.outside_apps;
    let answer = match (op, field("app")) {
        (AppOp::List, None) => Ok(list_all(state, &opened).await),
        (AppOp::List, Some(app)) => list_one(state, app, &opened).await,
        (AppOp::Call, Some(app)) => match field("action") {
            Some(action) => call(state, app, action, args.get("args"), &opened).await,
            None => Err("name the `action` to run".to_string()),
        },
        (AppOp::Call, None) => Err("name the `app` whose action to run".to_string()),
    };
    match answer {
        Ok(text) => tool_content(text, false),
        Err(why) => tool_content(why, true),
    }
}

async fn skill_named(state: &Arc<crate::server::ServerState>, app: &str) -> Result<Skill, String> {
    let found = state.manager.skills.get_skill(app).await;
    found.ok_or_else(|| crate::engine::skill::tools::Refusal::no_skill(app).to_string())
}

/// One line per app that shows an outside agent an action: its name, what
/// it is for, its actions' names.
async fn list_all(state: &Arc<crate::server::ServerState>, opened: &[String]) -> String {
    let skills = state.manager.skills.list_skills().await;
    let lines: Vec<String> = skills
        .iter()
        .filter_map(|skill| app_line(skill, host_tools(skill, opened).into_iter()))
        .map(
            |AppLine {
                 name,
                 about,
                 actions,
             }| format!("{name}: {about} Actions: {}.", actions.join(", ")),
        )
        .collect();
    match lines.is_empty() {
        true => "No app offers an outside agent an action right now.".to_string(),
        false => lines.join("\n"),
    }
}

/// One app's actions, as JSON lines — `AppTool`'s rows.
async fn list_one(
    state: &Arc<crate::server::ServerState>,
    app: &str,
    opened: &[String],
) -> Result<String, String> {
    let skill = skill_named(state, app).await?;
    open_to_host(&skill, opened).map_err(|why| why.to_string())?;
    let rows: Vec<String> = host_tools(&skill, opened)
        .into_iter()
        .map(|t| action_row(&skill, t).to_string())
        .collect();
    match rows.is_empty() {
        true => Ok(format!("'{app}' offers an outside agent no action.")),
        false => Ok(rows.join("\n")),
    }
}

/// Run one action; a command that exits non-zero is an error with its
/// output.
async fn call(
    state: &Arc<crate::server::ServerState>,
    app: &str,
    action: &str,
    args: Option<&Value>,
    opened: &[String],
) -> Result<String, String> {
    let skill = skill_named(state, app).await?;
    let tool_args = args.cloned().unwrap_or_else(|| serde_json::json!({}));
    if !tool_args.is_object() {
        return Err("`args` is the action's arguments, as an object".to_string());
    }
    match run_for_host(&skill, action, &tool_args, opened).await {
        Ok(ToolResult::CommandOutput {
            exit_code,
            stdout,
            stderr,
        }) => match exit_code {
            Some(0) | None => Ok(stdout),
            Some(code) => Err(format!("exit {code}: {}{stderr}", stdout)),
        },
        Ok(other) => Ok(format!("{other:?}")),
        Err(why) => Err(why.to_string()),
    }
}

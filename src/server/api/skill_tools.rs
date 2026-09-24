//! A skill page's declared door to its own scripts:
//!
//!   POST /api/skills/{skill}/tools/{tool}   body: the tool's args as JSON
//!   → { exit_code, stdout, stderr }
//!
//! Runs a shell (`cmd:`) tool the skill declares in its SKILL.md — exactly as
//! the model's call would run it: the same command template, PATH and
//! timeout (the child is killed when it runs out). Instead of a page writing
//! raw shell to `/api/bash`, it names a tool; the engine knows what may run.
//!
//! - one call per skill at a time (a per-skill lock), so two taps never
//!   interleave writes to the skill's files;
//! - the tool's `tier` must be within what the skill's `permission.paths`
//!   grants its working folder;
//! - a skill that keeps a cloud refuses while signed out (`AUTH_REQUIRED:`),
//!   like its turns, and a tool that may write pushes the save afterwards.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::engine::permission::PathMode;
use crate::engine::permission::{effective_mode_for_path, parse_skill_tier, PermissionMode};
use crate::engine::skill::Skill;
use crate::engine::skill_tool::{SkillToolDef, SkillToolKind};
use crate::engine::tools::ToolResult;
use crate::server::ServerState;

type SkillLock = Arc<tokio::sync::Mutex<()>>;

static SKILL_LOCKS: Mutex<Option<HashMap<String, SkillLock>>> = Mutex::new(None);

fn skill_lock(skill: &str) -> SkillLock {
    let mut guard = SKILL_LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get_or_insert_with(HashMap::new)
        .entry(skill.to_string())
        .or_default()
        .clone()
}

fn refuse(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, msg.into()).into_response()
}

pub async fn run_tool(
    State(state): State<Arc<ServerState>>,
    Path((skill_name, tool_name)): Path<(String, String)>,
    body: Option<Json<Value>>,
) -> Response {
    let args = body
        .map(|Json(v)| v)
        .unwrap_or_else(|| Value::Object(Default::default()));
    if !args.is_object() {
        return refuse(
            StatusCode::BAD_REQUEST,
            "the body is the tool's args, as an object",
        );
    }
    let Some(skill) = state.skills.get_skill(&skill_name).await else {
        return refuse(StatusCode::NOT_FOUND, format!("no skill '{skill_name}'"));
    };
    let tool = match shell_tool(&skill, &tool_name) {
        Ok(t) => t,
        Err(r) => return r,
    };
    if skill.cloud.is_some() && crate::account::resolve_token().is_none() {
        return refuse(StatusCode::UNAUTHORIZED, super::skill_cloud::AUTH_REQUIRED);
    }
    let cwd = working_folder(&skill);
    if let Err(r) = within_grant(&skill, &tool, &cwd) {
        return r;
    }

    let lock = skill_lock(&skill.name);
    let _one = lock.lock().await;
    let run_args = args.clone();
    let run_tool = tool.clone();
    let ran = tokio::task::spawn_blocking(move || run_tool.execute(&run_args, &cwd, &[])).await;
    let result = match ran {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return refuse(StatusCode::BAD_REQUEST, format!("{e:#}")),
        Err(e) => return refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")),
    };
    if may_write(&tool) {
        push_save(&skill).await;
    }
    Json(output(result)).into_response()
}

/// The declared shell tool, or why there is none.
fn shell_tool(skill: &Skill, name: &str) -> Result<SkillToolDef, Response> {
    let tool = skill
        .tool_defs
        .iter()
        .find(|t| t.name == name)
        .ok_or_else(|| {
            refuse(
                StatusCode::NOT_FOUND,
                format!("'{}' declares no tool '{name}'", skill.name),
            )
        })?;
    if tool.kind() != SkillToolKind::Shell {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            format!("'{name}' runs no command"),
        ));
    }
    let mut tool = tool.clone();
    tool.skill_name = Some(skill.name.clone());
    if tool.skill_dir.is_none() {
        tool.skill_dir = skill.skill_dir.clone();
    }
    Ok(tool)
}

/// Where the tool runs: the skill's declared `cwd`, else its own folder.
fn working_folder(skill: &Skill) -> PathBuf {
    let declared = skill
        .cwd
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    match (declared, skill.skill_dir.as_ref()) {
        (Some(cwd), _) => crate::util::resolve_path(std::path::Path::new(cwd)),
        (None, Some(dir)) => dir.clone(),
        (None, None) => std::env::temp_dir(),
    }
}

fn tier_of(tool: &SkillToolDef) -> PermissionMode {
    tool.tier
        .as_deref()
        .and_then(parse_skill_tier)
        .unwrap_or(PermissionMode::Admin)
}

fn may_write(tool: &SkillToolDef) -> bool {
    tier_of(tool) > PermissionMode::Read
}

/// The tool's tier must be covered by what the skill's own permission block
/// grants the folder it runs in — the scope its sessions get.
fn within_grant(skill: &Skill, tool: &SkillToolDef, cwd: &std::path::Path) -> Result<(), Response> {
    let grants: Vec<PathMode> = skill
        .permission
        .iter()
        .flat_map(|p| p.iter_grants())
        .map(|(path, mode)| PathMode {
            path: crate::util::resolve_path(std::path::Path::new(path))
                .to_string_lossy()
                .to_string(),
            mode,
        })
        .collect();
    let granted = effective_mode_for_path(&grants, cwd).unwrap_or(PermissionMode::Chat);
    let needed = tier_of(tool);
    if needed > granted {
        return Err(refuse(
            StatusCode::FORBIDDEN,
            format!(
                "'{}' needs {needed} on {}; the skill grants {granted}",
                tool.name,
                cwd.display()
            ),
        ));
    }
    Ok(())
}

/// A page's move changed the save: push it now, never pull (a pull happens
/// at a turn's edges and on the page's own sync).
async fn push_save(skill: &Skill) {
    let Some(save) = crate::account::cloud_save::target(skill) else {
        return;
    };
    tokio::spawn(async move {
        if let Err(e) = crate::account::cloud_save::push(&save).await {
            tracing::warn!("save '{}' not pushed after a page tool: {e:#}", save.skill);
        }
    });
}

fn output(result: ToolResult) -> Value {
    match result {
        ToolResult::CommandOutput {
            exit_code,
            stdout,
            stderr,
        } => serde_json::json!({ "exit_code": exit_code, "stdout": stdout, "stderr": stderr }),
        other => {
            serde_json::json!({ "exit_code": 0, "stdout": format!("{other:?}"), "stderr": "" })
        }
    }
}

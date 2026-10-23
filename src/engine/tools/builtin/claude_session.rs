//! Launch an independent SDK session, rather than a native delegated loop.
use super::{Tool, ToolCall, ToolResult, Tools};
use crate::engine::permission::PermissionMode;
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct ClaudeSessionTool;

#[async_trait]
impl Tool for ClaudeSessionTool {
    fn name(&self) -> &'static str {
        "StartClaudeSession"
    }
    fn description(&self) -> &'static str {
        "Start a separate Claude Agent SDK coding session. Include the goal, confirmed constraints and relevant context in task. Returns a session link; coding, approvals and follow-ups happen there."
    }
    // Launching a coding agent can write files and run commands after its
    // own SDK approvals, so this isn't a read-only messaging capability.
    fn tier(&self) -> PermissionMode {
        PermissionMode::Edit
    }
    fn args_schema(&self) -> Value {
        json!({"type":"object", "properties":{
            "title":{"type":"string", "description":"Short task title"},
            "task":{"type":"string", "description":"Self-contained task and relevant context"},
            "cwd":{"type":"string", "description":"Optional working folder; defaults to this session's folder"}
        }, "required":["title","task"]})
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({"name":self.name(), "args":{"title":"string", "task":"string", "cwd":"string?"}, "returns":"session_id, url"})
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        anyhow::ensure!(
            !tools
                .session_policy
                .as_ref()
                .is_some_and(|policy| policy.locked),
            "Claude sessions require the owner."
        );
        anyhow::ensure!(
            tools.ask_user_bridge.is_some(),
            "Session hosting is unavailable outside the running UI."
        );
        let manager = tools
            .get_manager()
            .context("Session hosting is unavailable")?;
        anyhow::ensure!(
            manager
                .models
                .read()
                .await
                .list_models()
                .iter()
                .any(|model| crate::server::claude::supports_model(model)),
            "Add a Claude API-key model in Settings → Models."
        );
        let title = call.args["title"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .context("A task title is required")?;
        let task = call.args["task"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .context("A task is required")?;
        let cwd = working_folder(&tools.cwd(), call.args["cwd"].as_str())?;
        let mut meta = crate::server::claude::new_meta(
            title.into(),
            cwd.to_string_lossy().into(),
            tools.session_id.clone(),
        );
        meta.title_locked = true;
        meta.user_id = tools
            .session_id
            .as_deref()
            .and_then(|id| manager.global_sessions.get_session_meta(id).ok().flatten())
            .and_then(|m| m.user_id);
        manager.global_sessions.add_session(&meta)?;
        manager
            .send_event(
                crate::engine::agent::AgentEvent::StartSession {
                    session_id: meta.id.clone(),
                    message: task.into(),
                    from: tools.agent_id().unwrap_or("ling").into(),
                },
                tools.session_id.clone(),
            )
            .await;
        manager
            .send_event(crate::engine::agent::AgentEvent::StateUpdated, None)
            .await;
        Ok(ToolResult::Success(json!({"session_id":meta.id, "url":format!("/?session={}",meta.id), "status":"started"}).to_string()))
    }
}

fn working_folder(base: &std::path::Path, chosen: Option<&str>) -> Result<std::path::PathBuf> {
    let path = match chosen.filter(|s| !s.is_empty()) {
        Some(value) if value.starts_with('~') || std::path::Path::new(value).is_absolute() => {
            crate::util::resolve_path(std::path::Path::new(value))
        }
        Some(value) => base.join(value),
        None => base.to_path_buf(),
    }
    .canonicalize()
    .context("Working folder does not exist")?;
    anyhow::ensure!(path.is_dir(), "Working folder must be a directory");
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_task_folders_belong_to_the_calling_session() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("child")).unwrap();
        assert_eq!(
            working_folder(dir.path(), Some(".")).unwrap(),
            dir.path().canonicalize().unwrap()
        );
        assert_eq!(
            working_folder(dir.path(), Some("child")).unwrap(),
            dir.path().join("child").canonicalize().unwrap()
        );
        assert!(working_folder(dir.path(), Some("absent")).is_err());
    }

    #[tokio::test]
    async fn a_consumer_cannot_launch_an_owner_sdk_session() {
        let dir = tempfile::tempdir().unwrap();
        let mut tools = Tools::new(dir.path().to_path_buf()).unwrap();
        tools.session_policy = Some(crate::engine::session_policy::SessionPolicy::consumer(
            Default::default(),
        ));
        let error = ClaudeSessionTool
            .execute(
                &tools,
                ToolCall {
                    tool: "StartClaudeSession".into(),
                    args: json!({"title":"task", "task":"hello"}),
                    block_id: None,
                },
            )
            .await
            .err()
            .unwrap();
        assert!(error.to_string().contains("require the owner"));
    }
}

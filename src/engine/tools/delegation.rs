use super::{AskUserBridge, ToolResult, Tools};
use crate::engine::agent::AgentManager;
use anyhow::Result;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub(crate) struct TaskArgs {
    pub(crate) target_agent_id: String,
    pub(crate) task: String,
    /// Run it beside the chat: the call returns at once, the end comes as a
    /// turn of its own (doc/background-tasks-spec.md).
    #[serde(default)]
    pub(crate) run_in_background: bool,
}

#[cfg(test)]
mod task_tests {
    use super::*;

    #[test]
    fn inherited_snapshot_replaces_defaults_preserves_overrides_and_cannot_escalate() {
        use crate::engine::permission::{check_permission, PathMode, PermissionCheckResult, PermissionMode, SessionPermissions};
        use std::path::Path;

        let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
        let private = cwd.join("private");
        for modes in [
            vec![
                PathMode { path: "*".into(), mode: PermissionMode::Admin },
                PathMode { path: private.to_string_lossy().into_owned(), mode: PermissionMode::Read },
            ],
            vec![
                PathMode { path: private.to_string_lossy().into_owned(), mode: PermissionMode::Read },
                PathMode { path: "*".into(), mode: PermissionMode::Admin },
            ],
        ] {
            let mut child = SessionPermissions::default();
            child.set_path_mode(&private.to_string_lossy(), PermissionMode::Admin);
            inherit_permissions(&mut child, &modes, false);
            assert!(!child.interactive);
            assert!(matches!(check_permission("Write", None, Some(private.join("x").to_str().unwrap()), cwd, &child, None), PermissionCheckResult::NeedsPrompt(_)));
            assert!(matches!(check_permission("Read", None, Some(private.join("x").to_str().unwrap()), cwd, &child, None), PermissionCheckResult::Allowed));
            let mut grandchild = SessionPermissions::default();
            inherit_permissions(&mut grandchild, &child.path_modes, child.interactive);
            assert!(!grandchild.interactive);
            assert!(matches!(check_permission("Write", None, Some(private.join("x").to_str().unwrap()), cwd, &grandchild, None), PermissionCheckResult::NeedsPrompt(_)));
        }
        let mut child = SessionPermissions::default();
        child.set_path_mode("*", PermissionMode::Admin);
        inherit_permissions(&mut child, &[], true);
        assert!(child.path_modes.is_empty());
        assert!(child.interactive, "a subagent of a person's session may ask");
        assert!(matches!(check_permission("Read", None, Some(cwd.join("x").to_str().unwrap()), cwd, &child, None), PermissionCheckResult::NeedsPrompt(_)));
    }

    #[tokio::test]
    async fn background_without_session_fails_before_spawning() {
        let tools = Tools::new(std::env::temp_dir()).unwrap();
        let args = TaskArgs {
            target_agent_id: "ling".into(),
            task: "test".into(),
            run_in_background: true,
        };
        let error = tools.validate_delegation(&args).err().unwrap().to_string();
        assert!(error.contains("run_in_background needs a session"), "{error}");
        let error = tools.task(args).await.unwrap_err().to_string();
        assert!(error.contains("run_in_background needs a session"), "{error}");
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct WebSearchArgs {
    #[serde(alias = "q")]
    pub(super) query: String,
    pub(super) max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct WebFetchArgs {
    pub(super) url: String,
    pub(super) max_bytes: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SkillArgs {
    #[serde(alias = "name")]
    pub(super) skill: String,
    pub(super) args: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RunAppArgs {
    #[serde(alias = "name")]
    pub(super) skill: String,
    pub(super) args: Option<String>,
}

impl Tools {
    /// Validate delegation policy/depth/target without executing.
    /// Returns the manager and caller agent id on success.
    pub(crate) fn validate_delegation(
        &self,
        args: &TaskArgs,
    ) -> Result<(Arc<AgentManager>, String)> {
        if args.run_in_background && self.session_id.as_deref().is_none_or(str::is_empty) {
            anyhow::bail!("run_in_background needs a session; run it in the foreground instead");
        }
        let manager = self
            .manager
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Delegation requires AgentManager context"))?;
        let caller_id = self
            .agent_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Delegation requires caller agent id"))?;

        if self.delegation_depth >= self.max_delegation_depth {
            anyhow::bail!(
                "Delegation denied: max delegation depth ({}) reached",
                self.max_delegation_depth
            );
        }

        Ok((manager.clone(), caller_id))
    }

    pub(super) async fn task(&self, args: TaskArgs) -> Result<ToolResult> {
        let (manager, caller_id) = self.validate_delegation(&args)?;
        if args.run_in_background {
            anyhow::bail!("run_in_background requires the session delegation dispatcher");
        }
        // Box::pin breaks the async-fn type cycle:
        // Tools::execute -> Tools::task -> run_delegation -> engine.run_agent_loop
        // -> ToolRegistry::execute -> Tools::execute. The recursion is real
        // (delegated agents run their own tool loops) but it goes through
        // a fresh engine, so the boxing only costs one heap allocation per
        // delegation.
        Box::pin(run_delegation(
            manager,
            self.root.clone(),
            caller_id,
            args.target_agent_id,
            args.task,
            self.run_id.clone(),
            self.delegation_depth,
            self.max_delegation_depth,
            self.ask_user_bridge.clone(),
            self.session_id.clone(),
            self.session_policy.clone(),
            self.parent_allowed_tools.clone(),
            self.parent_path_modes.clone(),
            self.parent_interactive,
            None,
        ))
        .await
    }

    pub(super) async fn run_app(&self, args: RunAppArgs) -> Result<ToolResult> {
        let manager = self
            .manager
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("RunApp requires AgentManager context"))?;

        let skill = manager.skills.get_skill(&args.skill).await;

        match skill {
            Some(skill) => {
                let app = skill.app.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "Skill '{}' does not have an app configuration. Only skills with 'app.launcher' can be run as apps.",
                        args.skill
                    )
                })?;

                match app.launcher.as_str() {
                    "web" => {
                        let url = format!("/apps/{}/{}", skill.name, app.entry);
                        // Emit AppLaunched event if we have the bridge.
                        if let Some(bridge) = &self.ask_user_bridge {
                            let _ = bridge.events_tx.send(
                                crate::engine::events::ServerEvent::AppLaunched {
                                    skill: skill.name.clone(),
                                    launcher: "web".to_string(),
                                    url: url.clone(),
                                    title: skill.description.clone(),
                                    width: app.width,
                                    height: app.height,
                                    session_id: bridge.session_id.clone(),
                                },
                            );
                        }
                        Ok(ToolResult::Success(format!(
                            "Launched web app '{}' at {}",
                            skill.name, url
                        )))
                    }
                    "url" => {
                        let url = app.entry.clone();
                        if let Some(bridge) = &self.ask_user_bridge {
                            let _ = bridge.events_tx.send(
                                crate::engine::events::ServerEvent::AppLaunched {
                                    skill: skill.name.clone(),
                                    launcher: "url".to_string(),
                                    url: url.clone(),
                                    title: skill.description.clone(),
                                    width: app.width,
                                    height: app.height,
                                    session_id: bridge.session_id.clone(),
                                },
                            );
                        }
                        Ok(ToolResult::Success(format!(
                            "Launched URL app '{}': {}",
                            skill.name, url
                        )))
                    }
                    "bash" => {
                        // Execute the entry script in the skill directory.
                        let skill_dir = skill.skill_dir.as_ref().ok_or_else(|| {
                            anyhow::anyhow!("Skill '{}' has no directory path", args.skill)
                        })?;
                        let script = skill_dir.join(&app.entry);
                        if !script.exists() {
                            anyhow::bail!("App script not found: {}", script.display());
                        }
                        let mut cmd = std::process::Command::new("sh");
                        cmd.arg(script.as_os_str());
                        if let Some(extra) = &args.args {
                            for arg in extra.split_whitespace() {
                                cmd.arg(arg);
                            }
                        }
                        cmd.current_dir(&self.root);
                        cmd.env("PATH", crate::util::shell_path());
                        let output = cmd
                            .output()
                            .map_err(|e| anyhow::anyhow!("Failed to run app script: {}", e))?;
                        Ok(ToolResult::command_output(
                            output.status.code(),
                            &String::from_utf8_lossy(&output.stdout),
                            &String::from_utf8_lossy(&output.stderr),
                            super::DEFAULT_MAX_TOOL_OUTPUT_BYTES,
                        ))
                    }
                    other => {
                        anyhow::bail!(
                            "Unknown app launcher type '{}'. Supported: web, bash, url.",
                            other
                        )
                    }
                }
            }
            None => {
                anyhow::bail!("Skill '{}' not found.", args.skill)
            }
        }
    }

    pub(super) async fn invoke_skill(&self, args: SkillArgs) -> Result<ToolResult> {
        let manager = self
            .manager
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Skill tool requires AgentManager context"))?;

        let skill = manager.skills.get_skill(&args.skill).await;

        match skill {
            Some(skill) => {
                if skill.disable_model_invocation {
                    anyhow::bail!(
                        "Skill '{}' is user-only (disable_model_invocation). It can only be invoked by the user via /{}.",
                        args.skill, args.skill
                    );
                }
                // Replace $SKILL_DIR in content so the model sees the actual path.
                let resolved_content = if let Some(ref dir) = skill.skill_dir {
                    skill.content.replace("$SKILL_DIR", &dir.to_string_lossy())
                } else {
                    skill.content.clone()
                };
                let mut content = format!(
                    "<skill-name>{}</skill-name>\n\n{}\n\n{}",
                    skill.name, skill.description, resolved_content
                );
                if let Some(ref extra_args) = args.args {
                    content.push_str(&format!("\n\nSkill arguments: {}", extra_args));
                }
                // NOTE: tool registration + active_skill mutation happens
                // at the engine's tool-dispatch layer, AFTER this result
                // is returned. See `engine/tool_exec.rs` post-`Skill`
                // hook: it calls `engine.activate_skill(_,
                // ActivationMode::ToolInvocation)` so the skill's tool
                // defs land in `engine.tools.skill_tools` for the next
                // turn. This method runs inside the `Tools` struct,
                // which has no engine back-reference — the dispatch
                // layer is the right home for engine state mutation.
                Ok(ToolResult::Success(content))
            }
            None => {
                let available = manager.skills.list_skills().await;
                let names: Vec<String> = available
                    .iter()
                    .filter(|s| !s.disable_model_invocation)
                    .map(|s| s.name.clone())
                    .collect();
                anyhow::bail!(
                    "Skill '{}' not found. Available skills: [{}]",
                    args.skill,
                    names.join(", ")
                );
            }
        }
    }
}

/// A delegated run starts with exactly the parent's path grants at dispatch
/// time. Beyond them it asks the person, as a Claude Code subagent's
/// permission prompt reaches the main view — only when the parent could ask:
/// with no one attached (a mission, a proxy consumer) a prompt would hang.
fn inherit_permissions(
    permissions: &mut crate::engine::permission::SessionPermissions,
    parent_path_modes: &[crate::engine::permission::PathMode],
    parent_interactive: bool,
) {
    permissions.path_modes = parent_path_modes.to_vec();
    permissions.interactive = parent_interactive;
}

#[derive(Debug)]
pub(crate) struct DelegationCancelled;

impl std::fmt::Display for DelegationCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Subagent cancelled")
    }
}

impl std::error::Error for DelegationCancelled {}

/// Execute a single delegation on a fresh, ephemeral engine.
///
/// This is a standalone async function (not a method) so it can be spawned onto
/// a `JoinSet` for parallel execution.  Each call creates its own `AgentEngine`
/// via `AgentManager::spawn_delegation_engine`, runs the agent loop, and drops
/// the engine when done.
pub(crate) async fn run_delegation(
    manager: Arc<AgentManager>,
    ws_root: PathBuf,
    caller_id: String,
    target_agent_id: String,
    task: String,
    parent_run_id: Option<String>,
    delegation_depth: usize,
    max_delegation_depth: usize,
    ask_user_bridge: Option<Arc<AskUserBridge>>,
    session_id: Option<String>,
    parent_policy: Option<crate::engine::session_policy::SessionPolicy>,
    parent_allowed_tools: Option<std::collections::HashSet<String>>,
    parent_path_modes: Vec<crate::engine::permission::PathMode>,
    parent_interactive: bool,
    started: Option<tokio::sync::oneshot::Sender<String>>,
) -> Result<ToolResult> {
    let run_id = manager
        .begin_delegated_run(
            &ws_root,
            session_id.as_deref(),
            &target_agent_id,
            parent_run_id.clone(),
            Some(format!("delegated by {}", caller_id)),
            started.is_some(),
        )
        .await?;
    if let Some(started) = started {
        let _ = started.send(run_id.clone());
    }

    manager
        .send_event(
            crate::engine::agent::AgentEvent::Message {
                from: caller_id.clone(),
                to: target_agent_id.clone(),
                content: format!("Delegated task: {}", task),
                // This synthetic "delegated" message comes from the caller
                // (depth-0 in the typical path), so it routes to main chat
                // as before. Subagent-originated terminal messages are the
                // ones that needed routing — they come from context.rs.
                run_id: None,
                parent_id: None,
            },
            session_id.clone(),
        )
        .await;

    manager
        .send_event(
            crate::engine::agent::AgentEvent::SubagentSpawned {
                parent_id: caller_id.clone(),
                subagent_id: target_agent_id.clone(),
                task: task.clone(),
                subagent_run_id: Some(run_id.clone()),
                parent_run_id: parent_run_id.clone(),
            },
            session_id.clone(),
        )
        .await;

    let engine_result = manager
        .spawn_delegation_engine(&ws_root, &target_agent_id)
        .await;
    let mut engine = match engine_result {
        Ok(e) => e,
        Err(err) => {
            let status = manager
                .finish_agent_run(
                    &run_id,
                    crate::engine::agent::AgentRunStatus::Failed,
                    Some(err.to_string()),
                )
                .await?;
            if status == crate::engine::agent::AgentRunStatus::Cancelled {
                return Err(DelegationCancelled.into());
            }
            return Err(err);
        }
    };

    engine.set_parent_agent(Some(caller_id.clone()));
    engine.set_parent_run_id(parent_run_id.clone());
    engine.set_delegation_depth(delegation_depth + 1, max_delegation_depth);
    engine.set_run_id(Some(run_id.clone()));
    engine.set_task(task);
    engine.tools.builtins.set_session_id(session_id.clone());
    engine.withhold_session_tools(&manager.global_sessions, session_id.as_deref());

    // Inherit consumer restrictions from parent — subagents can only tighten, never widen.
    if let Some(ref policy) = parent_policy {
        policy.apply(&mut engine);
    }
    // A separate ceiling survives policy and skill changes in the child.
    engine.cfg.delegated_allowed_tools = parent_allowed_tools;
    // A delegated run is a task whatever the parent's session was: the
    // parent's policy carries a person's-session profile (turns verbatim),
    // but the bootstrap message is the subagent's only delivery of its task.
    engine.prompt_profile.task_bootstrap = true;

    // Replace, rather than merge, the fresh engine's grants. Preserve the
    // snapshot verbatim: set_path_mode prunes more-specific entries and can
    // turn a wildcard or parent grant into an unintended elevation.
    inherit_permissions(&mut engine.session_permissions, &parent_path_modes, parent_interactive);
    engine.tools.builtins.parent_path_modes = parent_path_modes;
    engine.tools.builtins.parent_interactive = parent_interactive;

    // Wire AskUser bridge so the subagent can ask for permissions and questions.
    if let Some(bridge) = ask_user_bridge {
        engine.tools.set_ask_user_bridge(bridge);
    }

    let run_result = engine.run_agent_loop(session_id.as_deref()).await;
    // Capture sub-agent's last response before engine is dropped.
    let last_text = engine.last_assistant_text.take();

    let (outcome, status, detail) = match run_result {
        Ok(outcome) => (
            outcome,
            crate::engine::agent::AgentRunStatus::Completed,
            None,
        ),
        Err(err) => {
            let msg = err.to_string();
            let status = if msg.to_lowercase().contains("cancel") {
                crate::engine::agent::AgentRunStatus::Cancelled
            } else {
                crate::engine::agent::AgentRunStatus::Failed
            };
            let final_status = manager
                .finish_agent_run(&run_id, status, Some(msg.clone()))
                .await?;
            if final_status == crate::engine::agent::AgentRunStatus::Cancelled {
                return Err(DelegationCancelled.into());
            }
            return Err(err);
        }
    };
    let final_status = manager.finish_agent_run(&run_id, status, detail).await?;
    if final_status == crate::engine::agent::AgentRunStatus::Cancelled {
        return Err(DelegationCancelled.into());
    }

    manager
        .send_event(
            crate::engine::agent::AgentEvent::SubagentResult {
                parent_id: caller_id,
                subagent_id: target_agent_id,
                outcome: outcome.clone(),
                report: None,
                subagent_run_id: Some(run_id.clone()),
                parent_run_id: parent_run_id.clone(),
            },
            session_id.clone(),
        )
        .await;

    // When the sub-agent finished normally (AgentOutcome::None), surface its
    // last response text so the parent agent sees the actual result instead
    // of "agent_outcome: None".
    match outcome {
        crate::engine::AgentOutcome::None => {
            let text = last_text.unwrap_or_else(|| "Sub-agent completed.".to_string());
            Ok(ToolResult::Success(text))
        }
        other => Ok(ToolResult::AgentOutcome(other)),
    }
}

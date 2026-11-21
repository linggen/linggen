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
            (*self.turn_place).clone(),
            self.session_policy.clone(),
            self.parent_allowed_tools.clone(),
            self.parent_path_modes.clone(),
            self.parent_interactive,
            (self.parent_work, self.parent_ceiling, self.parent_read_origin),
            false,
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
                self.model_may_start(&skill).await?;
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
                        // It runs as Bash does (`run_command`): off the
                        // runtime's workers, in the session's folder, under
                        // the foreground limit, killed when the call is
                        // dropped.
                        let quote = crate::engine::skill_tool::shell_escape_arg;
                        let mut cmd = format!("sh {}", quote(&script.to_string_lossy()));
                        for arg in args.args.iter().flat_map(|a| a.split_whitespace()) {
                            cmd.push(' ');
                            cmd.push_str(&quote(arg));
                        }
                        self.run_command(super::search_exec::RunCommandArgs {
                            cmd,
                            timeout_ms: None,
                            run_in_background: false,
                            cancel_flag: None,
                        })
                        .await
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

    /// Whether the model may start `skill` — by Skill or by RunApp alike:
    /// never a user-only skill, nor one that keeps a session of its own out
    /// of the day's.
    async fn model_may_start(&self, skill: &crate::engine::skill::Skill) -> Result<()> {
        if skill.disable_model_invocation {
            anyhow::bail!(
                "Skill '{}' is user-only (disable_model_invocation). It can only be invoked by the user via /{}.",
                skill.name, skill.name
            );
        }
        if self.kept_out_of_day(skill) {
            anyhow::bail!(
                "refused: {} keeps a session of its own; it is not taken up in the day's session.",
                skill.name
            );
        }
        Ok(())
    }

    pub(super) async fn invoke_skill(&self, args: SkillArgs) -> Result<ToolResult> {
        let manager = self
            .manager
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Skill tool requires AgentManager context"))?;

        let skill = manager.skills.get_skill(&args.skill).await;

        match skill {
            Some(skill) => {
                self.model_may_start(&skill).await?;
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
    parent_place: Option<crate::engine::turn_place::TurnPlace>,
    parent_policy: Option<crate::engine::session_policy::SessionPolicy>,
    parent_allowed_tools: Option<std::collections::HashSet<String>>,
    parent_path_modes: Vec<crate::engine::permission::PathMode>,
    parent_interactive: bool,
    (parent_work, parent_ceiling, read_origin): (
        Option<crate::engine::session_spec::Work>,
        Option<crate::engine::permission::PermissionMode>,
        crate::engine::permission::ReadOrigin,
    ),
    background: bool,
    started: Option<tokio::sync::oneshot::Sender<String>>,
) -> Result<ToolResult> {
    let run_id = manager
        .begin_delegated_run(
            &ws_root,
            session_id.as_deref(),
            &target_agent_id,
            parent_run_id.clone(),
            Some(format!("delegated by {}", caller_id)),
            background,
        )
        .await?;
    let _life = manager.run_life(&run_id);
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
    // It stands where its parent's turn stands: the app's memory, values
    // and grants are the parent's place's; a private app's subagent keeps
    // its talk there too (`exit_gate`).
    if let Some(place) = &parent_place {
        let day_on = manager.daily_session();
        engine.tools.builtins.private_app = place.private_app(&*manager.skills, day_on).await;
    }
    engine.tools.builtins.stand_with_parent(parent_place);
    engine.withhold_session_tools(&manager.global_sessions, session_id.as_deref());

    // Inherit consumer restrictions from parent — subagents can only tighten,
    // never widen. A delegated run is a task whatever the parent's session
    // was: the bootstrap message is the subagent's only delivery of its task.
    let mut spec = crate::engine::session_spec::SessionSpec::delegated_child(
        parent_policy,
        parent_interactive,
        parent_work,
        parent_ceiling,
    );
    spec.read_origin = read_origin;
    spec.apply(&mut engine);
    // Under a guest's policy the child, like its parent, runs only a model
    // the room shares.
    engine
        .run_on_own_model(&format!("subagent '{target_agent_id}'"))
        .await;
    // A separate ceiling survives policy and skill changes in the child.
    engine.cfg.delegated_allowed_tools = parent_allowed_tools;

    // Replace, rather than merge, the fresh engine's grants. Preserve the
    // snapshot verbatim: set_path_mode prunes more-specific entries and can
    // turn a wildcard or parent grant into an unintended elevation.
    inherit_permissions(&mut engine.session_permissions, &parent_path_modes, parent_interactive);
    // One chat, one permission set: a subagent of a session uses the chat's
    // own grants, read live — the snapshot above stands only for a run with
    // no session to share.
    if let Some(ref sid) = session_id {
        engine.join_session_permissions(sid);
    }
    engine.tools.builtins.parent_path_modes = parent_path_modes;
    engine.tools.builtins.parent_interactive = parent_interactive;

    // Wire AskUser bridge so the subagent can ask for permissions and questions.
    if let Some(bridge) = ask_user_bridge {
        engine.tools.set_ask_user_bridge(bridge);
    }

    let run_result = engine.run_agent_loop(session_id.as_deref()).await;
    // A run ends only once its own background work has. The loop waited
    // for it; a run that ends early anyway (Stop, a failure) stops it here.
    crate::engine::background::end_owned(&manager, &run_id).await;
    // Capture sub-agent's last response before engine is dropped.
    let last_text = engine.last_assistant_text.take();

    let (outcome, status, detail) = match run_result {
        Ok(outcome) => (
            outcome,
            crate::engine::agent::AgentRunStatus::Completed,
            None,
        ),
        Err(err) => {
            // A failure is a failure: whether the run was cancelled is what
            // its own status says (`finish_agent_run` reads the cancel set
            // a Stop fills), never a word in the error.
            let final_status = manager
                .finish_agent_run(
                    &run_id,
                    crate::engine::agent::AgentRunStatus::Failed,
                    Some(err.to_string()),
                )
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

#[cfg(test)]
mod run_app_tests {
    use super::*;
    use crate::engine::skill::{Skill, SkillRegistry, SkillSource};
    use crate::engine::tools::ToolCall;
    use std::time::{Duration, Instant};

    /// A registry holding the given skills only.
    struct Installed(Vec<Skill>);

    #[async_trait::async_trait]
    impl SkillRegistry for Installed {
        async fn get_skill(&self, name: &str) -> Option<Skill> {
            self.0.iter().find(|s| s.name == name).cloned()
        }
        async fn reload_one(&self, name: &str) -> Option<Skill> {
            self.get_skill(name).await
        }
        async fn list_skills(&self) -> Vec<Skill> {
            self.0.clone()
        }
        async fn match_trigger(&self, _: &str) -> Option<(String, String)> {
            None
        }
        async fn list_metadata(&self) -> Vec<(String, String, bool)> {
            Vec::new()
        }
    }

    /// A skill whose `bash` app runs `script` (written to `run.sh` in `dir`).
    fn bash_app(dir: &std::path::Path, name: &str, extra: &str, script: &str) -> Skill {
        std::fs::write(dir.join("run.sh"), script).unwrap();
        let text = format!(
            "---\nname: {name}\ndescription: A test app.\n{extra}app:\n  launcher: bash\n  entry: run.sh\n---\nbody\n"
        );
        let mut skill =
            crate::extensions::skills::parse_skill_text(&text, SkillSource::Project).unwrap();
        skill.skill_dir = Some(dir.to_path_buf());
        skill
    }

    fn tools_with(dir: &std::path::Path, skills: Vec<Skill>) -> Tools {
        let (manager, _events) = AgentManager::new(
            crate::config::Config::default(),
            None,
            Arc::new(Installed(skills)),
            Arc::new(crate::engine::test_registries::Empty),
            Arc::new(crate::engine::test_registries::Empty),
            crate::engine::InterfaceMode::Web,
        );
        let mut tools = Tools::new(dir.to_path_buf()).unwrap();
        tools.set_context(manager, "ling".into());
        tools
    }

    fn run_app(skill: &str) -> ToolCall {
        ToolCall {
            tool: "RunApp".into(),
            args: serde_json::json!({ "skill": skill }),
            block_id: None,
        }
    }

    fn alive(pid: i32) -> bool {
        pid > 0 && unsafe { libc::kill(pid, 0) } == 0
    }

    /// A `bash` app runs like any other command the engine starts: off the
    /// runtime's workers, and killed when its call is dropped (Stop).
    #[tokio::test]
    async fn a_bash_app_is_killed_when_its_call_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("app.pid");
        let script = format!("echo $$ > '{}'\nexec sleep 5\n", pid_file.display());
        let tools = tools_with(dir.path(), vec![bash_app(dir.path(), "waiter", "", &script)]);
        let ran = tokio::time::timeout(Duration::from_millis(800), tools.execute(run_app("waiter"))).await;
        assert!(ran.is_err(), "the call held the runtime until its script ended: {ran:?}");
        let pid: i32 = std::fs::read_to_string(&pid_file)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        assert!(pid > 0, "the script never started");
        let deadline = Instant::now() + Duration::from_secs(5);
        while alive(pid) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let survived = alive(pid);
        if survived {
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        assert!(!survived, "the app's script outlived its call");
    }

    /// A user-only skill is refused to the model through RunApp as through
    /// Skill: one rule decides whether a model may start a skill.
    #[tokio::test]
    async fn a_user_only_app_is_refused_to_the_model() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("ran");
        let script = format!("touch '{}'\n", marker.display());
        let skill = bash_app(dir.path(), "private", "disable-model-invocation: true\n", &script);
        let tools = tools_with(dir.path(), vec![skill]);
        let err = tools.execute(run_app("private")).await.unwrap_err().to_string();
        assert!(err.contains("user-only"), "{err}");
        assert!(!marker.exists(), "the user-only app's script ran");
    }
}

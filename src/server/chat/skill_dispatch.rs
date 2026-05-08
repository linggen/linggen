use crate::server::chat::helpers::{emit_outcome_event, persist_and_emit_message};
use crate::server::ServerEvent;

use super::runtime::{
    push_user_turn_with_recall, run_loop_with_tracking, send_thinking_status,
    unwire_interrupt_channel, wire_ask_user_bridge, wire_interrupt_channel,
};
use super::ChatRunCtx;

/// Scope the agent's reachable-skill list to the active skill app's
/// `allow-skills` declaration. Mirrors the mission scoping path so skill
/// app sessions don't see every installed skill in the daemon — important
/// in the shared-daemon model where one ling serves many branded apps.
///
/// Default when `allow-skills` is unset: only the active skill itself.
/// `["*"]` opts out of scoping.
pub(super) fn apply_skill_app_scope(
    engine: &mut crate::engine::AgentEngine,
    skill: &crate::skills::Skill,
) {
    use std::collections::HashSet;
    let allow = skill.allow_skills.as_deref().unwrap_or(&[]);
    if allow.iter().any(|s| s == "*") {
        return;
    }
    let mut scoped: HashSet<String> = allow.iter().cloned().collect();
    scoped.insert(skill.name.clone());
    engine.cfg.consumer_allowed_skills = Some(scoped);
}

/// Dispatch a skill (slash command) invocation.
pub(super) async fn run_skill_dispatch(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
) {
    let parts: Vec<&str> = ctx.clean_msg.trim().splitn(2, ' ').collect();
    let cmd = parts[0].trim_start_matches('/');
    let user_args = parts
        .get(1)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(manager) = engine.tools.get_manager() {
        if let Some(skill) = manager.skill_manager.get_skill(cmd).await {
            if !skill.user_invocable {
                let err_msg = format!(
                    "Skill '{}' is not user-invocable and cannot be activated with /{cmd}.",
                    skill.name
                );
                persist_and_emit_message(
                    &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                    &ctx.agent_id, "user", &err_msg, ctx.session_id.as_deref(), false,
                )
                .await;
                return;
            }
            if !ctx.policy.is_skill_allowed(&skill.name) {
                let err_msg = format!("Skill '{}' is not available in this room.", skill.name);
                persist_and_emit_message(
                    &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                    &ctx.agent_id, "user", &err_msg, ctx.session_id.as_deref(), false,
                )
                .await;
                return;
            }
            // App skill: launch app UI only when --web flag is present.
            // Without --web, fall through to run as a regular skill (model uses tools).
            let wants_web = user_args.as_ref().map_or(false, |a| a.contains("--web"));
            if wants_web {
                if let Some(ref app) = skill.app {
                    let launch_msg = format!("Launching app: {}", skill.name);
                    persist_and_emit_message(
                        &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                        "user", &ctx.agent_id, &launch_msg, ctx.session_id.as_deref(), false,
                    )
                    .await;

                    match app.launcher.as_str() {
                        "web" => {
                            let url = format!("/apps/{}/{}", skill.name, app.entry);
                            let full_url = format!("http://localhost:{}{}", ctx.state.port, url);
                            let _ = ctx.events_tx.send(ServerEvent::AppLaunched {
                                skill: skill.name.clone(),
                                launcher: "web".to_string(),
                                url,
                                title: skill.description.clone(),
                                width: app.width,
                                height: app.height,
                                session_id: ctx.session_id.clone(),
                            });
                            if ctx.events_tx.receiver_count() <= 1 {
                                let _ = super::open_in_browser(&full_url);
                            }
                        }
                        "url" => {
                            let _ = ctx.events_tx.send(ServerEvent::AppLaunched {
                                skill: skill.name.clone(),
                                launcher: "url".to_string(),
                                url: app.entry.clone(),
                                title: skill.description.clone(),
                                width: app.width,
                                height: app.height,
                                session_id: ctx.session_id.clone(),
                            });
                            if ctx.events_tx.receiver_count() <= 1 {
                                let _ = super::open_in_browser(&app.entry);
                            }
                        }
                        "bash" => {
                            if let Some(ref skill_dir) = skill.skill_dir {
                                let script = skill_dir.join(&app.entry);
                                let mut cmd = std::process::Command::new("sh");
                                cmd.arg(script.as_os_str());
                                if let Some(ref args) = user_args {
                                    for arg in args.split_whitespace() {
                                        cmd.arg(arg);
                                    }
                                }
                                cmd.current_dir(&ctx.root);
                                match cmd.output() {
                                    Ok(output) => {
                                        let result_msg = String::from_utf8_lossy(&output.stdout).to_string();
                                        if !result_msg.trim().is_empty() {
                                            persist_and_emit_message(
                                                &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                                                &ctx.agent_id, "user", &result_msg, ctx.session_id.as_deref(), false,
                                            )
                                            .await;
                                        }
                                    }
                                    Err(e) => {
                                        let err_msg = format!("Failed to run app: {}", e);
                                        persist_and_emit_message(
                                            &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                                            &ctx.agent_id, "user", &err_msg, ctx.session_id.as_deref(), false,
                                        )
                                        .await;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                    let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
                    return;
                }
            }
            engine.active_skill = Some(skill);
        }
    }

    let skill_default_task = engine
        .active_skill
        .as_ref()
        .map(|s| format!("Run the '{}' skill: {}", s.name, s.description));
    let task_for_loop = user_args
        .or(skill_default_task)
        .unwrap_or_else(|| "Initialize this workspace and summarize status.".to_string());

    engine.observations.clear();
    engine.task = Some(task_for_loop);

    push_user_turn_with_recall(ctx, engine).await;

    let skill_msg = format!("Running skill: {}", cmd);
    persist_and_emit_message(
        &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
        &ctx.agent_id, "user", &skill_msg, ctx.session_id.as_deref(), false,
    )
    .await;

    tracing::info!("Skill started: {}", cmd);

    send_thinking_status(ctx, format!("Running skill: {}", cmd)).await;

    let interrupt_key = wire_interrupt_channel(ctx, engine).await;
    wire_ask_user_bridge(&ctx.state, engine, ctx.session_id.clone());

    // Hydrate session_permissions from permission.json BEFORE the prompt gate.
    // run_agent_loop normally does this, but it runs *after* the gate — so
    // without this preload, the gate sees the engine's default (unlocked)
    // state and re-prompts every turn even after the trusted policy was
    // persisted to disk.
    if let Some(ref sid) = ctx.session_id {
        let sdir = crate::paths::global_sessions_dir().join(sid);
        engine.session_permissions = crate::engine::permission::SessionPermissions::load(&sdir);
        if engine.session_dir.is_none() {
            engine.session_dir = Some(sdir);
        }
    }

    if !prompt_skill_permission_if_needed(ctx, engine).await {
        unwire_interrupt_channel(ctx, engine, &interrupt_key).await;
        return;
    }

    let outcome = run_loop_with_tracking(
        &ctx.manager, &ctx.root, engine, &ctx.agent_id,
        ctx.session_id.as_deref(), "chat:skill", &ctx.events_tx,
    )
    .await;

    unwire_interrupt_channel(ctx, engine, &interrupt_key).await;

    if let Err(e) = outcome {
        tracing::warn!("Skill loop failed: {}", e);
        let err_msg = format!("Error: {}", e);
        persist_and_emit_message(
            &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
            &ctx.agent_id, "user", &err_msg, ctx.session_id.as_deref(), false,
        )
        .await;
    } else {
        tracing::info!("Skill completed: {}", cmd);
        if let Ok(outcome) = &outcome {
            emit_outcome_event(outcome, &ctx.events_tx, &ctx.agent_id, ctx.session_id.as_deref());
        }
        let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
    }
}

/// Skill permission approval prompt — fires only when the engine has an
/// active skill that declares `permission` and the session is interactive.
/// Returns `false` if the user cancels (caller should abort the dispatch).
async fn prompt_skill_permission_if_needed(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
) -> bool {
    if !engine.session_permissions.interactive {
        return true;
    }
    let Some(skill) = engine.active_skill.clone() else {
        return true;
    };
    let Some(perm) = skill.permission.as_ref() else {
        return true;
    };

    let paths_str = perm.display_paths();
    let mut question_text = format!("Skill \"{}\" requests grants on: {}", skill.name, paths_str);
    if let Some(ref warning) = perm.warning {
        question_text.push_str(&format!("\n⚠️ {}", warning));
    }

    let question = crate::engine::tools::AskUserQuestion {
        question: question_text,
        header: "Permission".to_string(),
        options: vec![
            crate::engine::tools::AskUserOption {
                label: "Approve".to_string(),
                description: Some(format!("Grant: {}", paths_str)),
                preview: None,
            },
            crate::engine::tools::AskUserOption {
                label: "Run in current mode".to_string(),
                description: Some("Skill runs with existing permissions (may fail)".to_string()),
                preview: None,
            },
            crate::engine::tools::AskUserOption {
                label: "Cancel".to_string(),
                description: Some("Don't run this skill".to_string()),
                preview: None,
            },
        ],
        multi_select: false,
    };

    match engine.ask_permission_raw(&skill.name, question).await {
        Some(crate::engine::permission::PermissionAction::AllowOnce) => {
            // "Approve" — write each declared grant into session path_modes.
            for (path, mode) in perm.iter_grants() {
                engine.session_permissions.set_path_mode(path, mode);
            }
            if let Some(ref sdir) = engine.session_dir {
                engine.session_permissions.save(sdir);
            }
            true
        }
        Some(crate::engine::permission::PermissionAction::AllowSession) => {
            // "Run in current mode" — proceed without grants.
            true
        }
        _ => {
            // "Cancel" or timeout — abort skill.
            let msg = format!("Skill '{}' cancelled — permission not granted.", skill.name);
            persist_and_emit_message(
                &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                &ctx.agent_id, "user", &msg, ctx.session_id.as_deref(), false,
            )
            .await;
            false
        }
    }
}

/// Dispatch a user-defined trigger activation.
/// Similar to `run_skill_dispatch` but takes a pre-resolved skill name and remaining input.
pub(super) async fn run_trigger_dispatch(
    ctx: &ChatRunCtx,
    engine: &mut crate::engine::AgentEngine,
    skill_name: &str,
    remaining: &str,
) {
    let user_args = if remaining.is_empty() {
        None
    } else {
        Some(remaining.to_string())
    };

    let mut skill_default_task: Option<String> = None;
    if let Some(manager) = engine.tools.get_manager() {
        if let Some(skill) = manager.skill_manager.get_skill(skill_name).await {
            if !skill.user_invocable {
                let err_msg = format!(
                    "Skill '{}' is not user-invocable and cannot be activated via trigger.",
                    skill.name
                );
                persist_and_emit_message(
                    &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                    &ctx.agent_id, "user", &err_msg, ctx.session_id.as_deref(), false,
                )
                .await;
                return;
            }
            if !ctx.policy.is_skill_allowed(&skill.name) {
                let err_msg = format!("Skill '{}' is not available in this room.", skill.name);
                persist_and_emit_message(
                    &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                    &ctx.agent_id, "user", &err_msg, ctx.session_id.as_deref(), false,
                )
                .await;
                return;
            }
            if user_args.is_none() {
                skill_default_task =
                    Some(format!("Run the '{}' skill: {}", skill.name, skill.description));
            }
            engine.active_skill = Some(skill);
        }
    }

    let task_for_loop = user_args
        .or(skill_default_task)
        .unwrap_or_else(|| "Initialize this workspace and summarize status.".to_string());

    engine.observations.clear();
    engine.task = Some(task_for_loop);

    push_user_turn_with_recall(ctx, engine).await;

    let skill_msg = format!("Running skill via trigger: {}", skill_name);
    persist_and_emit_message(
        &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
        &ctx.agent_id, "user", &skill_msg, ctx.session_id.as_deref(), false,
    )
    .await;

    tracing::info!("Trigger skill started: {}", skill_name);

    send_thinking_status(ctx, format!("Running skill: {}", skill_name)).await;

    let interrupt_key = wire_interrupt_channel(ctx, engine).await;
    wire_ask_user_bridge(&ctx.state, engine, ctx.session_id.clone());

    let outcome = run_loop_with_tracking(
        &ctx.manager, &ctx.root, engine, &ctx.agent_id,
        ctx.session_id.as_deref(), "chat:trigger", &ctx.events_tx,
    )
    .await;

    unwire_interrupt_channel(ctx, engine, &interrupt_key).await;

    if let Err(e) = outcome {
        tracing::warn!("Trigger skill loop failed: {}", e);
        let err_msg = format!("Error: {}", e);
        persist_and_emit_message(
            &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
            &ctx.agent_id, "user", &err_msg, ctx.session_id.as_deref(), false,
        )
        .await;
    } else {
        tracing::info!("Trigger skill completed: {}", skill_name);
        if let Ok(outcome) = &outcome {
            emit_outcome_event(outcome, &ctx.events_tx, &ctx.agent_id, ctx.session_id.as_deref());
        }
        let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
    }
}

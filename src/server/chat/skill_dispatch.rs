use crate::engine::{ActivationMode, ActivationOutcome};
use crate::server::chat::helpers::{emit_outcome_event, persist_and_emit_message};
use crate::server::ServerEvent;

use super::runtime::{
    push_user_turn_with_recall, run_loop_with_tracking, send_thinking_status,
    unwire_interrupt_channel, wire_ask_user_bridge, wire_interrupt_channel,
};
use super::ChatRunCtx;

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

    // Resolve the skill, run policy/user_invocable checks, and handle
    // app-launcher (`--web`) flows that short-circuit before the agent
    // loop. Returns the resolved skill ready for activation, or `None`
    // when no skill matched the command (the loop still runs against
    // whatever `engine.active_skill` was previously set to).
    let resolved_skill = match resolve_slash_skill(ctx, engine, cmd, user_args.as_deref()).await {
        SlashResolution::Skill(skill) => Some(skill),
        SlashResolution::HandledOrBlocked => return,
        SlashResolution::NoMatch => None,
    };

    let skill_default_task = resolved_skill
        .as_ref()
        .map(|s| format!("Run the '{}' skill: {}", s.name, s.description));
    let task_for_loop = user_args
        .or(skill_default_task)
        .unwrap_or_else(|| "Initialize this workspace and summarize status.".to_string());

    engine.observations.clear();
    engine.task = Some(task_for_loop);

    // Hydrate session_permissions from permission.json BEFORE activate_skill
    // so the prompt's grant-comparison sees the trusted-policy state already
    // on disk. run_agent_loop hydrates later, but the prompt happens first
    // — without this preload it would re-prompt every turn.
    if let Some(ref sid) = ctx.session_id {
        let sdir = crate::paths::global_sessions_dir().join(sid);
        engine.session_permissions = crate::engine::permission::SessionPermissions::load(&sdir);
        if engine.session_dir.is_none() {
            engine.session_dir = Some(sdir);
        }
    }

    // Wire control channels before activation: activate_skill may prompt
    // the user (SlashCommand mode) and that flows through ask_permission_raw,
    // which needs the AskUser bridge in place.
    let interrupt_key = wire_interrupt_channel(ctx, engine).await;
    wire_ask_user_bridge(&ctx.state, engine, ctx.session_id.clone());

    if let Some(skill) = resolved_skill {
        // Declared permission.paths apply silently on activation (the
        // SKILL.md declaration is the approval); undeclared runtime access
        // still prompts via the per-operation ceiling check. So no grant
        // prompt fires here regardless of mode.
        match engine.activate_skill(skill, ActivationMode::SlashCommand).await {
            ActivationOutcome::Activated { grants_changed: true } => {
                let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
            }
            ActivationOutcome::Activated { grants_changed: false } => {}
            ActivationOutcome::Cancelled => {
                let msg = format!("Skill '{}' cancelled — permission not granted.", cmd);
                persist_and_emit_message(
                    &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
                    &ctx.agent_id, "user", &msg, ctx.session_id.as_deref(), false,
                )
                .await;
                unwire_interrupt_channel(ctx, engine, &interrupt_key).await;
                return;
            }
        }
    }

    push_user_turn_with_recall(ctx, engine).await;

    let skill_msg = format!("Running skill: {}", cmd);
    persist_and_emit_message(
        &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
        &ctx.agent_id, "user", &skill_msg, ctx.session_id.as_deref(), false,
    )
    .await;

    tracing::info!("Skill started: {}", cmd);
    send_thinking_status(ctx, format!("Running skill: {}", cmd)).await;

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

enum SlashResolution {
    /// Skill resolved and ready to activate.
    Skill(crate::extensions::skills::Skill),
    /// Either the `--web` app launcher fired (skill ran as an app, no agent
    /// loop), or the skill was blocked by user_invocable / policy checks
    /// (caller emitted the error and should `return`).
    HandledOrBlocked,
    /// No skill matched this command — the loop continues against whatever
    /// `engine.active_skill` was previously set to.
    NoMatch,
}

/// Resolve `/skill-name` against the skill manager. Runs policy/user_invocable
/// checks and the `--web` app-launcher branch (web/url/bash). Returns
/// [`SlashResolution`] describing what should happen next.
async fn resolve_slash_skill(
    ctx: &ChatRunCtx,
    engine: &crate::engine::AgentEngine,
    cmd: &str,
    user_args: Option<&str>,
) -> SlashResolution {
    let Some(manager) = engine.tools.get_manager() else {
        return SlashResolution::NoMatch;
    };
    let Some(skill) = manager.skills.reload_one(cmd).await else {
        return SlashResolution::NoMatch;
    };

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
        return SlashResolution::HandledOrBlocked;
    }
    if !ctx.policy.is_skill_allowed(&skill.name) {
        let err_msg = format!("Skill '{}' is not available in this room.", skill.name);
        persist_and_emit_message(
            &ctx.manager, &ctx.events_tx, &ctx.root, &ctx.agent_id,
            &ctx.agent_id, "user", &err_msg, ctx.session_id.as_deref(), false,
        )
        .await;
        return SlashResolution::HandledOrBlocked;
    }

    // App skill: launch app UI only when --web flag is present. Without --web,
    // fall through to run as a regular skill (model uses tools).
    let wants_web = user_args.is_some_and(|a| a.contains("--web"));
    if wants_web {
        if let Some(ref app) = skill.app {
            launch_skill_app(ctx, &skill, app, user_args).await;
            return SlashResolution::HandledOrBlocked;
        }
    }

    SlashResolution::Skill(skill)
}

/// Handle the three `--web` app-launcher variants (web/url/bash) for a
/// skill that declares an `app:` block.
async fn launch_skill_app(
    ctx: &ChatRunCtx,
    skill: &crate::extensions::skills::Skill,
    app: &crate::extensions::skills::AppConfig,
    user_args: Option<&str>,
) {
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
                if let Some(args) = user_args {
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
        if let Some(skill) = manager.skills.reload_one(skill_name).await {
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
            // Trigger mode is implicit-approval: prefix opt-in stands in for
            // the prompt. activate_skill never returns Cancelled here.
            engine.activate_skill(skill, ActivationMode::Trigger).await;
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

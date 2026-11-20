use crate::engine::{ActivationMode, ActivationOutcome};
use crate::server::chat::helpers::{emit_outcome_event, persist_and_emit_message};
use crate::server::ServerEvent;

use super::runtime::{
    push_user_turn_with_recall, run_loop_with_tracking, send_thinking_status,
    unwire_inbox, wire_engine_bridges, wire_inbox,
};
use super::ChatRunCtx;

/// Dispatch a skill (slash command) invocation.
pub(super) async fn run_skill_dispatch(ctx: &ChatRunCtx, engine: &mut crate::engine::AgentEngine) {
    let (cmd, user_args) = slash_command(&ctx.clean_msg);
    let user_args = user_args.map(str::to_string);

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

    engine.begin_run(crate::engine::turn_place::RunSetup::task(task_for_loop));

    // Hydrate session_permissions from permission.json BEFORE activate_skill
    // so the prompt's grant-comparison sees the trusted-policy state already
    // on disk. run_agent_loop hydrates later, but the prompt happens first
    // — without this preload it would re-prompt every turn.
    if let Some(ref sid) = ctx.session_id {
        let place = engine.turn_place().and_then(|p| p.grants_key()).map(str::to_string);
        engine.read_session_grants(sid, place.as_deref());
    }

    // Wire control channels before activation: activate_skill may prompt
    // the user (SlashCommand mode) and that flows through ask_permission_raw,
    // which needs the AskUser bridge in place.
    wire_inbox(ctx, engine);
    wire_engine_bridges(&ctx.state, engine, ctx.session_id.clone());

    if let Some(skill) = resolved_skill {
        // Declared permission.paths apply silently on activation (the
        // SKILL.md declaration is the approval); undeclared runtime access
        // still prompts via the per-operation ceiling check. So no grant
        // prompt fires here regardless of mode.
        let name = skill.name.clone();
        match engine.activate_skill(skill, ActivationMode::SlashCommand).await {
            ActivationOutcome::Activated { grants_changed: true } => {
                let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
            }
            ActivationOutcome::Activated { .. } => {}
            ActivationOutcome::Refused => {
                unwire_inbox(ctx, engine);
                say_kept_apart(ctx, &name).await;
                return;
            }
        }
    }

    push_user_turn_with_recall(ctx, engine).await;

    let skill_msg = format!("Running skill: {}", cmd);
    persist_and_emit_message(
        &ctx.manager,
        &ctx.events_tx,
        &ctx.root,
        &ctx.agent_id,
        &ctx.agent_id,
        "user",
        &skill_msg,
        ctx.session_id.as_deref(),
        false,
        None,
    )
    .await;

    tracing::info!("Skill started: {}", cmd);
    send_thinking_status(ctx, format!("Running skill: {}", cmd)).await;

    let outcome = run_loop_with_tracking(
        &ctx.manager,
        &ctx.root,
        engine,
        &ctx.agent_id,
        ctx.session_id.as_deref(),
        "chat:skill",
        &ctx.events_tx,
    )
    .await;

    unwire_inbox(ctx, engine);

    if let Err(e) = outcome {
        tracing::warn!("Skill loop failed: {}", e);
        super::helpers::end_failed_turn(ctx, engine.last_run_id.as_deref(), &e).await;
    } else {
        tracing::info!("Skill completed: {}", cmd);
        if let Ok(outcome) = &outcome {
            emit_outcome_event(ctx, outcome, engine.last_run_id.as_deref()).await;
        }
        let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
    }
}

/// A slash command's name and its arguments, from the message without the
/// `@name` that addressed it. The day's carry (`day_session`) reads it the
/// same way.
pub(crate) fn slash_command(words: &str) -> (&str, Option<&str>) {
    let mut parts = words.trim().splitn(2, ' ');
    let cmd = parts.next().unwrap_or("").trim_start_matches('/');
    let args = parts.next().map(str::trim).filter(|s| !s.is_empty());
    (cmd, args)
}

/// The line a turn shows when the day's session keeps the app it named out.
async fn say_kept_apart(ctx: &ChatRunCtx, skill: &str) {
    let line = format!("{skill} keeps a session of its own; open it from its page.");
    persist_and_emit_message(
        &ctx.manager,
        &ctx.events_tx,
        &ctx.root,
        &ctx.agent_id,
        &ctx.agent_id,
        "user",
        &line,
        ctx.session_id.as_deref(),
        false,
        None,
    )
    .await;
}

/// How a person started a skill: the one difference in what the gate says.
#[derive(Clone, Copy)]
enum UserStart<'a> {
    /// `/name`, as typed.
    Slash(&'a str),
    /// A trigger prefix the person set up.
    Trigger,
}

/// Whether the person may start `skill` here — by `/name` or by a trigger
/// alike: never one the skill keeps from users (`user-invocable: false`),
/// nor one this room does not offer. `Err` is the line the turn shows. The
/// day's session keeping a private app out is decided after this, in
/// `activate_skill`; the model's side is `model_may_start` (`delegation.rs`).
/// Locked by `the_user_side_skill_gate_is_written_once`.
fn user_may_start(
    skill: &crate::extensions::skills::Skill,
    policy: &crate::engine::session_policy::SessionPolicy,
    how: UserStart<'_>,
) -> Result<(), String> {
    if !skill.user_invocable {
        let by = match how {
            UserStart::Slash(cmd) => format!("with /{cmd}"),
            UserStart::Trigger => "via trigger".to_string(),
        };
        return Err(format!(
            "Skill '{}' is not user-invocable and cannot be activated {by}.",
            skill.name
        ));
    }
    if !policy.is_skill_allowed(&skill.name) {
        return Err(format!("Skill '{}' is not available in this room.", skill.name));
    }
    Ok(())
}

/// The line a turn shows when [`user_may_start`] refuses.
async fn say_refused(ctx: &ChatRunCtx, line: &str) {
    persist_and_emit_message(
        &ctx.manager,
        &ctx.events_tx,
        &ctx.root,
        &ctx.agent_id,
        &ctx.agent_id,
        "user",
        line,
        ctx.session_id.as_deref(),
        false,
        None,
    )
    .await;
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

    if let Err(line) = user_may_start(&skill, &ctx.policy, UserStart::Slash(cmd)) {
        say_refused(ctx, &line).await;
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
        &ctx.manager,
        &ctx.events_tx,
        &ctx.root,
        &ctx.agent_id,
        "user",
        &ctx.agent_id,
        &launch_msg,
        ctx.session_id.as_deref(),
        false,
        None,
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
                cmd.env("PATH", crate::util::shell_path());
                cmd.env(crate::language::ENV_KEY, crate::language::effective());
                match cmd.output() {
                    Ok(output) => {
                        let result_msg = String::from_utf8_lossy(&output.stdout).to_string();
                        if !result_msg.trim().is_empty() {
                            persist_and_emit_message(
                                &ctx.manager,
                                &ctx.events_tx,
                                &ctx.root,
                                &ctx.agent_id,
                                &ctx.agent_id,
                                "user",
                                &result_msg,
                                ctx.session_id.as_deref(),
                                false,
                                None,
                            )
                            .await;
                        }
                    }
                    Err(e) => {
                        let err_msg = format!("Failed to run app: {}", e);
                        persist_and_emit_message(
                            &ctx.manager,
                            &ctx.events_tx,
                            &ctx.root,
                            &ctx.agent_id,
                            &ctx.agent_id,
                            "user",
                            &err_msg,
                            ctx.session_id.as_deref(),
                            false,
                            None,
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
            if let Err(line) = user_may_start(&skill, &ctx.policy, UserStart::Trigger) {
                say_refused(ctx, &line).await;
                return;
            }
            if user_args.is_none() {
                skill_default_task = Some(format!(
                    "Run the '{}' skill: {}",
                    skill.name, skill.description
                ));
            }
            // Trigger mode is implicit-approval: prefix opt-in stands in for
            // the prompt. A day's session keeps a private app out.
            let name = skill.name.clone();
            let outcome = engine.activate_skill(skill, ActivationMode::Trigger).await;
            if matches!(outcome, ActivationOutcome::Refused) {
                say_kept_apart(ctx, &name).await;
                return;
            }
        }
    }

    let task_for_loop = user_args
        .or(skill_default_task)
        .unwrap_or_else(|| "Initialize this workspace and summarize status.".to_string());

    engine.begin_run(crate::engine::turn_place::RunSetup::task(task_for_loop));

    push_user_turn_with_recall(ctx, engine).await;

    let skill_msg = format!("Running skill via trigger: {}", skill_name);
    persist_and_emit_message(
        &ctx.manager,
        &ctx.events_tx,
        &ctx.root,
        &ctx.agent_id,
        &ctx.agent_id,
        "user",
        &skill_msg,
        ctx.session_id.as_deref(),
        false,
        None,
    )
    .await;

    tracing::info!("Trigger skill started: {}", skill_name);

    send_thinking_status(ctx, format!("Running skill: {}", skill_name)).await;

    wire_inbox(ctx, engine);
    wire_engine_bridges(&ctx.state, engine, ctx.session_id.clone());

    let outcome = run_loop_with_tracking(
        &ctx.manager,
        &ctx.root,
        engine,
        &ctx.agent_id,
        ctx.session_id.as_deref(),
        "chat:trigger",
        &ctx.events_tx,
    )
    .await;

    unwire_inbox(ctx, engine);

    if let Err(e) = outcome {
        tracing::warn!("Trigger skill loop failed: {}", e);
        super::helpers::end_failed_turn(ctx, engine.last_run_id.as_deref(), &e).await;
    } else {
        tracing::info!("Trigger skill completed: {}", skill_name);
        if let Ok(outcome) = &outcome {
            emit_outcome_event(ctx, outcome, engine.last_run_id.as_deref()).await;
        }
        let _ = ctx.events_tx.send(ServerEvent::StateUpdated);
    }
}

#[cfg(test)]
mod tests {
    /// This file's code, its tests left out.
    fn code() -> &'static str {
        let all = include_str!("skill_dispatch.rs");
        all.split("#[cfg(test)]").next().unwrap_or(all)
    }

    use super::{user_may_start, UserStart};
    use crate::engine::session_policy::{ConsumerCeiling, SessionPolicy};
    use crate::extensions::skills::{parse_skill_text, SkillSource};

    fn skill(invocable: bool) -> crate::extensions::skills::Skill {
        let text = format!(
            "---\nname: dice\ndescription: Dice.\nuser-invocable: {invocable}\n---\nBody."
        );
        parse_skill_text(&text, SkillSource::Global).expect("skill")
    }

    /// Both ways a person starts a skill meet the same two refusals.
    #[test]
    fn a_person_may_start_only_an_offered_user_invocable_skill() {
        let owner = SessionPolicy::owner();
        let room = SessionPolicy::consumer(ConsumerCeiling {
            skills: vec!["other".into()],
            ..Default::default()
        });
        for how in [UserStart::Slash("dice"), UserStart::Trigger] {
            assert_eq!(user_may_start(&skill(true), &owner, how), Ok(()));
            let kept = user_may_start(&skill(false), &owner, how).unwrap_err();
            assert!(kept.contains("not user-invocable"), "{kept}");
            let not_here = user_may_start(&skill(true), &room, how).unwrap_err();
            assert!(not_here.contains("not available in this room"), "{not_here}");
        }
    }

    /// The lock on `user_may_start`: a person's `/name` and a trigger ask
    /// the same gate, so each of its checks is written once in this file.
    #[test]
    fn the_user_side_skill_gate_is_written_once() {
        for check in [".user_invocable", ".is_skill_allowed("] {
            assert_eq!(
                code().matches(check).count(),
                1,
                "`{check}` is checked in more than one place; ask `user_may_start`"
            );
        }
    }
}

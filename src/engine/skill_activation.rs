//! Skill activation: the single entry point for entering a skill context.
//!
//! Replaces the four ad-hoc `engine.active_skill = Some(...)` write sites
//! that used to live in `server/chat/{handler, skill_dispatch, admin}` and
//! consolidates the four different "should I prompt? should I apply
//! grants?" rules into one place ([`ActivationMode`]).

use crate::engine::permission::effective_mode_for_path;
use crate::engine::AgentEngine;
use crate::engine::skill::Skill;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// What kind of activation event is happening — controls whether to prompt
/// the user, whether to apply declared grants, and whether the engine
/// should treat this as a real session change vs. a read-only export.
#[derive(Debug, Clone, Copy)]
pub enum ActivationMode {
    /// Session was created bound to this skill (skill-embed sessions, the
    /// ling-mem dashboard, etc.). Implicit approval — apply grants
    /// silently when the session is interactive; non-interactive sessions
    /// (mission/consumer) keep their existing tier.
    SessionBound,
    /// User typed `/skill-name`. Prompt the user to approve grants on
    /// interactive sessions; non-interactive sessions activate without
    /// prompting (skill runs at the session's existing tier).
    SlashCommand,
    /// User input matched a registered trigger prefix (e.g. `/commit ...`
    /// when a skill claims `trigger: "/commit"`). Implicit approval — the
    /// prefix opt-in stands in for the prompt.
    Trigger,
    /// Agent invoked the skill via the built-in `Skill` tool. Tools-only
    /// activation: register the skill's tool defs into the session and
    /// set `active_skill`, but DO NOT prompt for grants and DO NOT write
    /// `session_permissions`. The Skill tool returns the SKILL.md content
    /// as a tool result; activation here is the side effect that makes
    /// the skill's tools callable on the next turn.
    ToolInvocation,
    /// Read-only — for `get_system_prompt_api`. Sets `active_skill` on a
    /// throwaway engine so prompt export reflects the skill, but never
    /// writes session_permissions, never saves to disk, never emits events.
    Export,
}

#[derive(Debug)]
pub enum ActivationOutcome {
    /// Skill is now active. `grants_changed` indicates whether
    /// `session_permissions` was modified — the caller should emit
    /// `ServerEvent::StateUpdated` so the UI's permission badge refreshes.
    Activated { grants_changed: bool },
    /// User cancelled the permission prompt (only possible for
    /// `SlashCommand` on interactive sessions). Caller should abort
    /// dispatch and surface a "skill cancelled" message.
    Cancelled,
}

impl AgentEngine {
    /// Enter a skill context. See [`ActivationMode`] for the per-mode rules.
    ///
    /// Sets:
    /// - `consumer_allowed_skills` (via [`apply_skill_app_scope`]) — scope
    ///   the agent's reachable skill list to what this skill declared.
    /// - `session_permissions` (when grants apply per the mode + interactive
    ///   gating) — stamp each `permission.paths` entry into `path_modes`,
    ///   skipping entries that already cover the path at the same tier.
    /// - `active_skill` — committed last so a `Cancelled` outcome leaves
    ///   the engine unchanged.
    pub async fn activate_skill(
        &mut self,
        skill: Skill,
        mode: ActivationMode,
    ) -> ActivationOutcome {
        // Export: throwaway engine — set active_skill + scope, no grant side
        // effects, no save, no prompt. Used by the Copy-System-Prompt button.
        // Tools NOT registered on export — the export path is a read-only
        // prompt snapshot and the throwaway engine's tool registry is
        // discarded.
        if matches!(mode, ActivationMode::Export) {
            apply_skill_app_scope(self, &skill);
            self.active_skill = Some(skill);
            return ActivationOutcome::Activated { grants_changed: false };
        }

        // ToolInvocation: agent-driven Skill tool call. Tools-only mode —
        // register skill tool defs and set active_skill, but never prompt
        // for grants and never write permissions. Cwd seeding is also
        // skipped because the agent is borrowing the skill's tools, not
        // entering its workspace.
        if matches!(mode, ActivationMode::ToolInvocation) {
            register_skill_tools(self, &skill);
            apply_skill_app_scope(self, &skill);
            self.active_skill = Some(skill);
            return ActivationOutcome::Activated { grants_changed: false };
        }

        // All remaining real activations (SessionBound, Trigger, and
        // SlashCommand) apply the skill's DECLARED permission.paths grants
        // silently — the SKILL.md declaration is the approval, so there is
        // no activation-time prompt. Anything the skill tries to touch at
        // runtime BEYOND its declared grants still hits the per-operation
        // ceiling check in `permission::check_permission`, which prompts
        // ("Switch this folder to … / Allow once / Deny"). Declared = no
        // ask; undeclared = ask. Grants are only written for interactive
        // sessions (headless missions don't persist grants).
        let grants_changed = if self.session_permissions.interactive {
            write_skill_grants(self, &skill)
        } else {
            false
        };
        register_skill_tools(self, &skill);
        apply_skill_app_scope(self, &skill);
        apply_skill_tool_scope(self, &skill);
        seed_session_cwd_from_skill(self, &skill);
        self.active_skill = Some(skill);
        ActivationOutcome::Activated { grants_changed }
    }
}

/// Register a skill's tool defs into the engine's `skill_tools` registry.
/// Stamps the owning skill's name + dir on each entry so dispatch can
/// resolve back without a second lookup.
///
/// **Accumulation policy:** tools from earlier-activated skills stay
/// loaded. The agent keeps access to historical skill tools throughout
/// the session.
///
/// **Collision policy:** if two skills declare a tool with the same
/// name, the newer activation overwrites the older entry and a warning
/// is logged. No skill ships colliding names today; the warning is a
/// future tripwire.
fn register_skill_tools(engine: &mut AgentEngine, skill: &Skill) {
    if skill.disable_model_invocation {
        return;
    }
    for tool_def in &skill.tool_defs {
        if let Some(existing) = engine.tools.skill_tools.get(&tool_def.name) {
            let prev_owner = existing.skill_name.as_deref().unwrap_or("<unknown>");
            tracing::warn!(
                "skill tool name collision: '{}' from skill '{}' overwrites entry from '{}'",
                tool_def.name,
                skill.name,
                prev_owner,
            );
        }
        let mut def = tool_def.clone();
        def.skill_name = Some(skill.name.clone());
        if def.skill_dir.is_none() {
            def.skill_dir = skill.skill_dir.clone();
        }
        engine.tools.register_skill_tool(def);
    }
}

/// Seed the engine's per-session cwd to the skill's declared `cwd:` when
/// the session doesn't already have one. Without this, permission checks
/// on skill tools (FetchReddit, etc.) resolve against the request's
/// `project_root` — usually $HOME — and trip the ExceedsCeiling prompt
/// asking for read access on the whole home directory, even though the
/// skill's own SKILL.md grant already covers its actual workspace.
fn seed_session_cwd_from_skill(engine: &AgentEngine, skill: &Skill) {
    let Some(cwd_str) = skill.cwd.as_deref() else {
        tracing::debug!("[skill-activate] {} has no cwd declared — skipping seed", skill.name);
        return;
    };
    let trimmed = cwd_str.trim();
    if trimmed.is_empty() {
        return;
    }
    let expanded: PathBuf = if trimmed == "~" {
        let Some(home) = dirs::home_dir() else { return };
        home
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        let Some(home) = dirs::home_dir() else { return };
        home.join(rest)
    } else {
        PathBuf::from(trimmed)
    };
    let prev_cwd = engine.tools.builtins.cwd();
    engine.tools.builtins.seed_session_cwd_if_unset(expanded.clone());
    let post_cwd = engine.tools.builtins.cwd();
    tracing::info!(
        "[skill-activate] {} cwd seed: declared={} expanded={} prev={} post={}",
        skill.name,
        cwd_str,
        expanded.display(),
        prev_cwd.display(),
        post_cwd.display(),
    );
}

/// Scope the agent's reachable-skill list to the active skill's
/// `allow-skills` declaration. Mirrors the mission scoping path so skill
/// app sessions don't see every installed skill in the daemon — important
/// in the shared-daemon model where one ling serves many branded apps.
///
/// Default when `allow-skills` is unset: only the active skill itself.
/// `["*"]` opts out of scoping.
fn apply_skill_app_scope(engine: &mut AgentEngine, skill: &Skill) {
    let allow = skill.allow_skills.as_deref().unwrap_or(&[]);
    if allow.iter().any(|s| s == "*") {
        return;
    }
    let mut scoped: HashSet<String> = allow.iter().cloned().collect();
    scoped.insert(skill.name.clone());
    engine.cfg.consumer_allowed_skills = Some(scoped);
}

/// Restrict the session's tool surface to the skill's declared
/// `allowed-tools`. ling supplies the personality/soul; the active skill
/// supplies the tools. An empty/absent list means no restriction (inherit
/// ling's full set). Only for session-entering activations (SessionBound /
/// SlashCommand / Trigger) — NOT the transient `ToolInvocation` borrow,
/// where ling keeps its own tools, nor `Export` (read-only snapshot).
///
/// `allowed-tools` gates only the shared ENGINE tools. A skill's OWN
/// provided tools — its `tools:` capabilities plus the auto-injected
/// `PageUpdate` for app-skills — are intrinsic to the skill and are always
/// available, so they're unioned into the scope. (Without this an app
/// skill that declares `allowed-tools` could never render its dashboard.)
fn apply_skill_tool_scope(engine: &mut AgentEngine, skill: &Skill) {
    let mut scope =
        crate::extensions::scope::compute_tool_scope(skill.allowed_tools.as_deref().unwrap_or(&[]));
    if let Some(set) = scope.as_mut() {
        for td in &skill.tool_defs {
            set.insert(td.name.clone());
        }
    }
    engine.cfg.skill_allowed_tools = scope;
}

/// Stamp a skill's declared `permission.paths` grants into the engine's
/// `session_permissions`, saving to disk if `session_dir` is set.
///
/// Returns `true` when at least one grant changed the existing tier (UI
/// should refresh its permission badge). Returns `false` if the skill has
/// no permission block or every declared path is already covered at the
/// requested tier.
///
/// Intentionally does NOT broaden to cwd: the skill gets exactly what its
/// SKILL.md declared in `permission.paths` and nothing more. Auto-
/// broadening to cwd was previously done so the badge reflected the
/// skill's tier, but it silently inflated grants when cwd was a parent of
/// declared paths (e.g. cwd=~ + paths=[~/.linggen] → admin on whole
/// home).
fn write_skill_grants(engine: &mut AgentEngine, skill: &Skill) -> bool {
    let Some(perm) = skill.permission.as_ref() else { return false };
    let mut changed = false;
    for (path, mode) in perm.iter_grants() {
        let current = effective_mode_for_path(
            &engine.session_permissions.path_modes,
            Path::new(path),
        );
        if current != Some(mode) {
            engine.session_permissions.set_path_mode(path, mode);
            changed = true;
        }
    }
    if changed {
        if let Some(ref sdir) = engine.session_dir {
            engine.session_permissions.save(sdir);
        }
    }
    changed
}


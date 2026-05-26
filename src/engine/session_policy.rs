//! Session policy — single source of truth for consumer permission restrictions.
//!
//! Built once from the user type, then applied to the engine.
//! Every enforcement point reads from the same policy instead of
//! re-deriving the answer from scattered config fields.
//!
//! Two user types:
//! - **Owner** (`user_type = "owner"`): no restrictions, full prompt, full control.
//! - **Consumer** (`user_type = "consumer"`): room_config ceiling, locked session, restricted prompt.
//!
//! Session mode (admin/edit/read/chat) is a separate axis — it controls what
//! the agent can do, but for consumers it operates within the room_config ceiling.

use std::collections::HashSet;

/// Encapsulates all permission decisions for a session run.
///
/// Built from user type via `from_user_type()`, applied to the engine
/// via `apply()`. Skill checks use `is_skill_allowed()`. Tool checks
/// go through `EngineConfig::is_tool_allowed()` (the engine enforcement layer).
#[derive(Debug, Clone)]
pub struct SessionPolicy {
    /// Tools the session is allowed to use. None = unrestricted (owner).
    pub allowed_tools: Option<HashSet<String>>,
    /// Skills the session is allowed to invoke. None = unrestricted (owner).
    pub allowed_skills: Option<HashSet<String>>,
    /// When true, the agent never prompts the user for permissions.
    pub locked: bool,
    /// Prompt profile — which system prompt sections to include.
    pub prompt_profile: super::prompt::profile::PromptProfile,
}

impl SessionPolicy {
    /// Owner policy — no restrictions, full prompt.
    pub fn owner() -> Self {
        Self {
            allowed_tools: None,
            allowed_skills: None,
            locked: false,
            prompt_profile: super::prompt::profile::PromptProfile::owner(),
        }
    }

    /// Consumer policy — room_config ceiling, locked session, restricted prompt.
    pub fn consumer() -> Self {
        let room_cfg = crate::server::rtc::room_config::load_room_config();
        Self {
            allowed_tools: Some(room_cfg.allowed_tools.into_iter().collect()),
            allowed_skills: Some(room_cfg.allowed_skills.into_iter().collect()),
            locked: true,
            prompt_profile: super::prompt::profile::PromptProfile::consumer(),
        }
    }

    /// Build policy from the user type string injected by peer.rs.
    /// - `"owner"` → no restrictions.
    /// - `"consumer"` → room_config ceiling, locked.
    pub fn from_user_type(user_type: &str) -> Self {
        match user_type {
            "consumer" => Self::consumer(),
            _ => Self::owner(),
        }
    }

    /// Check if a skill is allowed by this policy.
    pub fn is_skill_allowed(&self, name: &str) -> bool {
        self.allowed_skills
            .as_ref()
            .map_or(true, |s| s.contains(name))
    }

    /// Apply this policy to an engine — the ONLY place engine mutation happens.
    /// Always applies, even for owner — clears any stale consumer restrictions.
    /// Also stores the policy on tools so subagents inherit it via delegation.
    pub fn apply(&self, engine: &mut super::AgentEngine) {
        engine.cfg.consumer_allowed_tools = self.allowed_tools.clone();
        engine.cfg.consumer_allowed_skills = self.allowed_skills.clone();
        // Consumer sessions are non-interactive (no prompts surface). Owner
        // sessions stay interactive. Mission sessions are flipped separately
        // by the mission scheduler.
        engine.session_permissions.interactive = !self.locked;
        engine.prompt_profile = self.prompt_profile.clone();
        // Store on tools for subagent propagation.
        engine.tools.builtins.session_policy = Some(self.clone());
        // Only invalidate cached prompt when restrictions change.
        if self.allowed_tools.is_some() || self.allowed_skills.is_some() || self.locked {
            engine.cached_system_prompt = None;
        }
    }
}

impl Default for SessionPolicy {
    fn default() -> Self {
        Self::owner()
    }
}

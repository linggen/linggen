//! Session policy — single source of truth for consumer permission restrictions.
//!
//! Built once from the user type, then applied to the engine.
//! Every enforcement point reads from the same policy instead of
//! re-deriving the answer from scattered config fields.
//!
//! Three kinds of asker (the server's `chat::policy::turn_policy` picks):
//! - **Owner**: no restrictions, full prompt, full control.
//! - **Person** of the household (another paired phone): the owner's session
//!   and prompt, held to the apps and models the owner gave them.
//! - **Consumer** (a room's guest): room_config ceiling, locked session, restricted prompt.
//!
//! Session mode (admin/edit/read/chat) is a separate axis — it controls what
//! the agent can do, but for a person or a consumer it operates within the ceiling.

use std::collections::HashSet;

/// What the owner lets a consumer use — the daemon reads it from its room
/// config; the engine only applies it.
#[derive(Debug, Clone, Default)]
pub struct ConsumerCeiling {
    pub tools: Vec<String>,
    pub skills: Vec<String>,
    /// The models the room shares (`shared_models`).
    pub models: Vec<String>,
}

/// Encapsulates all permission decisions for a session run.
///
/// Built by the server's `turn_policy`, carried to the engine by
/// its session spec (`session_spec::SessionSpec::apply`). Skill checks use `is_skill_allowed()`. Tool checks
/// go through `EngineConfig::is_tool_allowed()` (the engine enforcement layer).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionPolicy {
    /// Tools the session is allowed to use. None = unrestricted (owner).
    pub allowed_tools: Option<HashSet<String>>,
    /// Skills the session is allowed to invoke. None = unrestricted (owner).
    pub allowed_skills: Option<HashSet<String>>,
    /// Models the session may run on, as the room lists them. None =
    /// unrestricted (owner). Read through [`Self::asker`].
    pub allowed_models: Option<Vec<String>>,
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
            allowed_models: None,
            locked: false,
            prompt_profile: super::prompt::profile::PromptProfile::owner(),
        }
    }

    /// Consumer policy — the owner's ceiling, locked session, restricted prompt.
    pub fn consumer(ceiling: ConsumerCeiling) -> Self {
        Self {
            allowed_tools: Some(ceiling.tools.into_iter().collect()),
            allowed_skills: Some(ceiling.skills.into_iter().collect()),
            allowed_models: Some(ceiling.models),
            locked: true,
            prompt_profile: super::prompt::profile::PromptProfile::consumer(),
        }
    }

    /// A household person's policy — the owner's session and prompt, held to
    /// the apps and the models the owner gave them. Their tools are decided by
    /// the level their turn stands at (`SessionSpec::typed`), not here.
    pub fn person(skills: Vec<String>, models: Vec<String>) -> Self {
        Self {
            allowed_tools: None,
            allowed_skills: Some(skills.into_iter().collect()),
            allowed_models: Some(models),
            locked: false,
            prompt_profile: super::prompt::profile::PromptProfile::owner(),
        }
    }

    /// Who asks for a model under this policy (`model_choice`).
    pub fn asker(&self) -> crate::engine::model_choice::Asker<'_> {
        match &self.allowed_models {
            Some(shared) => crate::engine::model_choice::Asker::Guest(shared),
            None => crate::engine::model_choice::Asker::Owner,
        }
    }

    /// Check if a skill is allowed by this policy.
    pub fn is_skill_allowed(&self, name: &str) -> bool {
        self.allowed_skills
            .as_ref()
            .is_none_or(|s| s.contains(name))
    }

    /// A person's turn under this policy, as a test sets an engine up: the
    /// engine's spec, typed by this policy (`SessionSpec::typed`).
    #[cfg(test)]
    pub fn apply(&self, engine: &mut super::AgentEngine) {
        let policy = self.clone();
        engine.respec(|spec| {
            let tools = spec.tools.clone();
            *spec = spec
                .clone()
                .typed(policy, super::session_spec::Memory::Full, None);
            spec.tools = tools;
        });
    }
}

impl Default for SessionPolicy {
    fn default() -> Self {
        Self::owner()
    }
}

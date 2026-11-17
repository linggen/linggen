//! Writing a spec onto an engine: the one writer of a session's prompt
//! profile, tool restriction, interactivity, world and framing. A turn's
//! place narrows the turn on top of it (`turn_place::bind`) and puts back
//! what it narrowed when the turn ends.

use super::{Framing, Permission, SessionSpec};
use crate::engine::prompt::profile::PromptProfile;
use crate::engine::AgentEngine;
use std::collections::HashSet;
use std::path::PathBuf;

impl SessionSpec {
    /// Make this the engine's spec: its prompt profile, tool restriction,
    /// who answers beyond the grants, its world and its framing.
    pub fn apply(&self, engine: &mut AgentEngine) {
        self.write(engine, None);
    }

    /// The prompt profile this spec gives: the asking person's sections (the
    /// owner's when none claimed the run), its memory, its framing.
    pub fn profile(&self) -> PromptProfile {
        let mut profile = match &self.policy {
            Some(policy) => policy.prompt_profile.clone(),
            None => PromptProfile::owner(),
        };
        self.memory.narrow(&mut profile);
        profile.task_bootstrap = self.framing == Framing::Task;
        profile
    }

    fn write(&self, engine: &mut AgentEngine, kept: Option<PlaceScope>) {
        let mut profile = self.profile();
        if let Some(kept) = kept {
            kept.restore(&mut profile);
        }
        let tools = self.tools.allowlist();
        let (consumer_tools, consumer_skills) = self.consumer_sets();
        let changed = engine.session_spec.work != self.work
            || engine.prompt_profile != profile
            || engine.cfg.mission_allowed_tools != tools
            || engine.cfg.consumer_allowed_tools != consumer_tools
            || engine.cfg.consumer_allowed_skills != consumer_skills;
        engine.prompt_profile = profile;
        engine.cfg.mission_allowed_tools = tools;
        engine.cfg.consumer_allowed_tools = consumer_tools;
        engine.cfg.consumer_allowed_skills = consumer_skills;
        engine.session_permissions.interactive = self.permission == Permission::Interactive;
        engine.session_permissions.ask_caller = self.permission == Permission::AskCaller;
        engine.session_creator = self.creator.clone();
        // Subagents inherit the asking person's policy through delegation.
        engine.tools.builtins.session_policy = self.policy.clone();
        engine.tools.builtins.parent_work = self.work;
        if changed {
            engine.cached_system_prompt = None;
        }
        engine.session_spec = self.clone();
    }

    fn consumer_sets(&self) -> (Option<HashSet<String>>, Option<HashSet<String>>) {
        match &self.policy {
            Some(policy) => (policy.allowed_tools.clone(), policy.allowed_skills.clone()),
            None => (None, None),
        }
    }
}

impl AgentEngine {
    /// The spec this engine runs under.
    pub fn session_spec(&self) -> &SessionSpec {
        &self.session_spec
    }

    /// Change this engine's spec and apply it. The memory scope a place
    /// already entered this turn stays (the place puts it back itself).
    pub fn respec(&mut self, change: impl FnOnce(&mut SessionSpec)) {
        let mut spec = self.session_spec.clone();
        change(&mut spec);
        let kept = PlaceScope::of(&self.prompt_profile);
        spec.write(self, Some(kept));
    }

    /// After a loop loads the session's `permission.json`: a restricted
    /// session (a consumer's, a mission's, a delegate's) never prompts the
    /// person, whatever the file says.
    pub(crate) fn settle_loaded_permissions(&mut self) {
        if self.cfg.consumer_allowed_tools.is_some() || self.cfg.mission_allowed_tools.is_some() {
            self.session_permissions.interactive = false;
        }
    }
}

/// The memory scope a turn's place sets on the profile (`turn_place::bind`).
struct PlaceScope {
    memory_context: Option<String>,
    memory_app_dir: Option<PathBuf>,
    memory_recall_min_score: Option<f32>,
    memory_recall_count: Option<usize>,
}

impl PlaceScope {
    fn of(profile: &PromptProfile) -> Self {
        Self {
            memory_context: profile.memory_context.clone(),
            memory_app_dir: profile.memory_app_dir.clone(),
            memory_recall_min_score: profile.memory_recall_min_score,
            memory_recall_count: profile.memory_recall_count,
        }
    }

    fn restore(self, profile: &mut PromptProfile) {
        profile.memory_context = self.memory_context;
        profile.memory_app_dir = self.memory_app_dir;
        profile.memory_recall_min_score = self.memory_recall_min_score;
        profile.memory_recall_count = self.memory_recall_count;
    }
}

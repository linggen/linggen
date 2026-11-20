//! Writing a spec onto an engine: the one writer of a session's prompt
//! profile, tool restriction, interactivity, world and framing. A turn's
//! place narrows the turn on top of it (`turn_place::bind`) and puts back
//! what it narrowed when the turn ends.

use super::{Framing, Permission, SessionSpec, Work};
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
            || engine.session_spec.creator != self.creator
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
        engine.tools.builtins.delegated_session = self.preset == super::Preset::Delegate
            || matches!(self.tools, super::Tools::Delegate { .. });
        engine.tools.builtins.parent_ceiling = self.ceiling;
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

    /// Put a mission in charge of this engine: its body is the prompt's
    /// runbook, and `change` makes the spec its run's (its tools, its
    /// memory, who answers). A prompt cached before predates the body.
    pub fn take_up_mission(
        &mut self,
        mission: crate::engine::ActiveMission,
        change: impl FnOnce(&mut SessionSpec),
    ) {
        self.active_mission = Some(mission);
        self.respec(change);
        self.cached_system_prompt = None;
    }

    /// How much of the memory this engine's member recalls per turn: `count`
    /// rows at no less than `min_score` (`None`: the store's floor). A
    /// member's own, in any session it speaks in; the configured recall
    /// until a member tunes it (the companion's, from her settings).
    pub fn tune_recall(&mut self, count: usize, min_score: Option<f32>) {
        self.cfg.memory_recall_count = count;
        self.cfg.memory_inject_min_score = min_score;
    }

    /// Whether a mission is in charge of this engine.
    pub fn runs_mission(&self) -> bool {
        self.active_mission.is_some()
    }

    /// Whether this engine runs a coding session: its spec's work is `code`
    /// (a preset that takes the tag, or a `Task` child of such a session),
    /// and the session's stored tag still says so — turning Coding off takes
    /// effect at the next permission check, not only at the next turn.
    pub(crate) fn is_coding_session(&self) -> bool {
        if self.session_spec.work != Some(Work::Code) {
            return false;
        }
        match self.session_dir.as_deref() {
            Some(dir) => crate::state_fs::sessions::stored_work(dir) == Some(Work::Code),
            None => true,
        }
    }

    /// How many steps (a model reply and its tool calls) one turn of this
    /// engine may take: a coding session's own limit, else the chat limit.
    /// The one place that says so; the loop reads it once per turn.
    pub(crate) fn step_limit(&self) -> usize {
        if self.is_coding_session() {
            self.cfg.coding_max_iters
        } else {
            self.cfg.max_iters
        }
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

//! Prompt profile — declares which system prompt sections to include.
//!
//! Set once by `SessionPolicy::apply()`, read by the prompt builder.
//! No scattered if/else — the profile is the single decision point.

/// Which prompt sections to include for this session.
#[derive(Debug, Clone)]
pub struct PromptProfile {
    /// Environment block: ws_root, shell, platform.
    pub include_environment: bool,
    /// Project context files: CLAUDE.md, AGENTS.md, .cursorrules.
    pub include_project_context: bool,
    /// Memory: project MEMORY.md + global MEMORY.md.
    pub include_memory: bool,
    /// Core memory (who the person is) in an app's session, where
    /// `include_memory` is off: an app's turn knows the person, and reads
    /// nothing else but its own memory (Hanli, 2026-10-16: 「lingjing里面应该
    /// 有lingjing相关记忆. 当然还有core记忆」 · 「只是相应app相关的, 不要加载进来其他的」).
    /// Off for missions and consumers.
    pub include_core: bool,
    /// Frame the turn as an autonomous task: wrap the task text in the
    /// bootstrap message (workspace listing + step-by-step coda). Off for a
    /// person's session — their message reaches the model verbatim, the way
    /// a chat turn does in Claude Code. On for missions, delegated subagents
    /// and headless agent runs, where the input really is a task.
    pub task_bootstrap: bool,
    /// Available agents for Task delegation.
    pub include_delegation: bool,
    /// Consumer-specific frame: explains constraints to the model.
    pub consumer_frame: bool,
    /// Scoped per-app memory: set when the bound skill declares
    /// `memory-context` (its opt-in to memory). The turn auto-recalls ONLY the
    /// rows under `memory_app_dir` (`~/.linggen/skills/<name>`) and the prompt
    /// carries that dir's index — independent of `include_memory` (which stays
    /// off for skill sessions: core rides via `include_core`, nothing else of
    /// the biography). The two `recall_*` fields tune that scoped recall;
    /// `None` → engine defaults.
    pub memory_context: Option<String>,
    pub memory_app_dir: Option<std::path::PathBuf>,
    pub memory_recall_min_score: Option<f32>,
    pub memory_recall_count: Option<usize>,
}

impl PromptProfile {
    /// Owner — a person's session: full prompt, all sections, turns verbatim.
    pub fn owner() -> Self {
        Self {
            include_environment: true,
            include_project_context: true,
            include_memory: true,
            include_core: true,
            task_bootstrap: false,
            include_delegation: true,
            consumer_frame: false,
            memory_context: None,
            memory_app_dir: None,
            memory_recall_min_score: None,
            memory_recall_count: None,
        }
    }

    /// Consumer — restricted prompt, no owner-private sections.
    pub fn consumer() -> Self {
        Self {
            include_environment: false,
            include_project_context: false,
            include_memory: false,
            include_core: false,
            task_bootstrap: false,
            include_delegation: false,
            consumer_frame: true,
            memory_context: None,
            memory_app_dir: None,
            memory_recall_min_score: None,
            memory_recall_count: None,
        }
    }

    /// Narrow memory to who opened the session (`handler::turn_creator`):
    /// a person's keeps everything; an app's (`skill`) keeps core and its own
    /// memory only; a mission's keeps neither.
    pub fn narrow_for_creator(&mut self, creator: &str) {
        if creator == "user" {
            return;
        }
        self.include_memory = false;
        if creator == "mission" {
            self.include_core = false;
        }
    }

    /// Scope memory to a bound skill's own dir when it declares
    /// `memory-context` — the one wiring a real turn and the prompt export
    /// share. A skill without it leaves the profile as is.
    pub fn scope_to_app(&mut self, skill: &crate::engine::skill::Skill) {
        let opted_in = skill
            .memory_context
            .as_deref()
            .is_some_and(|c| !c.trim().is_empty());
        if !opted_in {
            return;
        }
        self.memory_context = skill.memory_context.clone();
        self.memory_app_dir = Some(crate::paths::global_skills_dir().join(&skill.name));
        self.memory_recall_min_score = skill.memory_recall_min_score;
        self.memory_recall_count = skill.memory_recall_count;
    }

    /// Autonomous run — a mission, a delegated subagent, a headless agent
    /// run: the owner's sections, and the task framed as a task.
    pub fn autonomous() -> Self {
        Self {
            task_bootstrap: true,
            ..Self::owner()
        }
    }
}

/// An engine that no session policy has claimed is running a task, not
/// talking to a person: missions and headless runs set `task` directly and
/// never pass through `SessionPolicy::apply()`.
impl Default for PromptProfile {
    fn default() -> Self {
        Self::autonomous()
    }
}

#[cfg(test)]
mod tests {
    use super::PromptProfile;

    /// An app's session knows the person (core) and nothing else of the
    /// biography; a mission knows neither; a person's keeps everything.
    #[test]
    fn memory_narrows_by_who_opened_the_session() {
        let mut app = PromptProfile::owner();
        app.narrow_for_creator("skill");
        assert!(!app.include_memory && app.include_core);
        let mut mission = PromptProfile::owner();
        mission.narrow_for_creator("mission");
        assert!(!mission.include_memory && !mission.include_core);
        let mut person = PromptProfile::owner();
        person.narrow_for_creator("user");
        assert!(person.include_memory && person.include_core);
        assert!(!PromptProfile::consumer().include_core);
    }
}

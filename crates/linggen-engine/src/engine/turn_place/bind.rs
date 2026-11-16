//! Entering and leaving a turn's place: everything a turn takes from the
//! app it stands in is set by [`AgentEngine::enter_place`] when the turn
//! starts and put back by [`AgentEngine::leave_place`] when it ends — the
//! skill's instructions, its tools, the folder, the memory it recalls, the
//! session values its commands read. The engine outlives the turn; nothing
//! of one place may reach a turn in another.

use super::TurnPlace;
use crate::engine::skill::Skill;
use crate::engine::skill_activation::{ActivationMode, ActivationOutcome};
use crate::engine::AgentEngine;
use std::collections::HashSet;
use std::path::PathBuf;

/// What entering a place changed, kept so leaving puts exactly that back.
#[derive(Debug, Default)]
pub(crate) struct Entered {
    /// The skill tools the place registered that were not there before.
    registered: Vec<String>,
    /// The scopes before the skill's were applied — `None` when the place
    /// has no skill and so set none.
    scopes: Option<Scopes>,
    /// Where the session stood before a place other than its own moved
    /// it to its own folder.
    folder: Option<Folder>,
}

/// Where a session's turns stand: its folder, and the workspace root the
/// prompt (environment, project instructions) and the file tools read
/// from it.
#[derive(Debug)]
struct Folder {
    /// The session's folder; `None` when it had none of its own yet.
    cwd: Option<PathBuf>,
    ws_root: PathBuf,
    tools_root: PathBuf,
    memory_cwd: Option<PathBuf>,
}

/// The engine's settings a skill's place overrides.
#[derive(Debug)]
struct Scopes {
    skill_allowed_tools: Option<HashSet<String>>,
    consumer_allowed_skills: Option<HashSet<String>>,
    tools_active_skill: Option<(String, PathBuf)>,
    memory: MemoryScope,
}

/// The parts of the prompt profile that say which memory a turn recalls.
#[derive(Debug, Clone)]
struct MemoryScope {
    include_memory: bool,
    memory_context: Option<String>,
    memory_app_dir: Option<PathBuf>,
    memory_recall_min_score: Option<f32>,
    memory_recall_count: Option<usize>,
}

impl MemoryScope {
    fn of(engine: &AgentEngine) -> Self {
        let p = &engine.prompt_profile;
        Self {
            include_memory: p.include_memory,
            memory_context: p.memory_context.clone(),
            memory_app_dir: p.memory_app_dir.clone(),
            memory_recall_min_score: p.memory_recall_min_score,
            memory_recall_count: p.memory_recall_count,
        }
    }

    fn restore(self, engine: &mut AgentEngine) {
        let p = &mut engine.prompt_profile;
        p.include_memory = self.include_memory;
        p.memory_context = self.memory_context;
        p.memory_app_dir = self.memory_app_dir;
        p.memory_recall_min_score = self.memory_recall_min_score;
        p.memory_recall_count = self.memory_recall_count;
    }
}

impl AgentEngine {
    /// Stand this engine's next turn in `place`. `skill` is the place's
    /// skill, resolved and allowed by the caller (`None` for home, or when
    /// it is gone). Whatever place the engine stood in before is left
    /// first. `mode` is [`ActivationMode::SessionBound`] for a real turn and
    /// [`ActivationMode::Export`] for the prompt export.
    pub async fn enter_place(
        &mut self,
        place: TurnPlace,
        skill: Option<Skill>,
        mode: ActivationMode,
    ) -> ActivationOutcome {
        self.leave_place();
        let mut entered = Entered {
            folder: self.move_to_place_folder(&place, skill.as_ref()),
            ..Entered::default()
        };
        self.tools.builtins.turn_place = Some(place);
        self.cached_system_prompt = None;
        let Some(skill) = skill else {
            self.entered_place = Some(entered);
            return ActivationOutcome::Activated {
                grants_changed: false,
            };
        };
        entered.scopes = Some(Scopes {
            skill_allowed_tools: self.cfg.skill_allowed_tools.clone(),
            consumer_allowed_skills: self.cfg.consumer_allowed_skills.clone(),
            tools_active_skill: self.tools.builtins.active_skill.clone(),
            memory: MemoryScope::of(self),
        });
        // An app's turn reads no biography: core rides via `include_core`,
        // and the app's own memory when it declares `memory-context`.
        self.prompt_profile.include_memory = false;
        self.prompt_profile.scope_to_app(&skill);
        let before: HashSet<String> = self.tools.skill_tools.keys().cloned().collect();
        let outcome = self.activate_skill(skill, mode).await;
        entered.registered = self
            .tools
            .skill_tools
            .keys()
            .filter(|name| !before.contains(*name))
            .cloned()
            .collect();
        self.entered_place = Some(entered);
        outcome
    }

    /// Put back what [`enter_place`](Self::enter_place) changed: the
    /// engine stands nowhere until its next turn enters a place. A no-op
    /// when it stands nowhere already.
    pub fn leave_place(&mut self) {
        let Some(entered) = self.entered_place.take() else {
            return;
        };
        self.active_skill = None;
        self.skill_bound = false;
        self.tools.builtins.turn_place = None;
        self.tools.builtins.turn_sight = None;
        self.cached_system_prompt = None;
        for name in &entered.registered {
            self.tools.skill_tools.remove(name);
        }
        if let Some(scopes) = entered.scopes {
            self.cfg.skill_allowed_tools = scopes.skill_allowed_tools;
            self.cfg.consumer_allowed_skills = scopes.consumer_allowed_skills;
            self.tools.builtins.active_skill = scopes.tools_active_skill;
            scopes.memory.restore(self);
        }
        if let Some(folder) = entered.folder {
            self.back_to_folder(folder);
        }
        self.back_to_own_grants();
    }

    /// What the running turn's place may be given of the session's rows,
    /// kept until the turn leaves its place (`turn_place::sight`).
    pub fn see_from_place(&mut self, sight: super::sight::Sight) {
        self.tools.builtins.turn_sight = Some(sight);
    }

    /// The place a summary written in the running turn is kept to: its own,
    /// when its talk is hidden from the session's other places; `None` —
    /// everyone's — otherwise.
    pub(crate) fn summary_kept_to(&self, meta: &crate::state_fs::sessions::SessionMeta) -> Option<String> {
        let place = self.turn_place()?;
        let sight = self.tools.builtins.turn_sight.as_ref()?;
        sight
            .hides_from_others(place, meta)
            .then(|| place.key().to_string())
    }

    /// The key of the slot the running turn's grants live in, in the
    /// session's `permission.json`: `None` for the session's own place.
    pub(crate) fn grants_place(&self) -> Option<String> {
        self.turn_place()?.grants_key().map(str::to_string)
    }

    /// Grants held for another place go back to the session's own once its
    /// turn is over: no grant of one place is left to the next turn.
    fn back_to_own_grants(&mut self) {
        if self.session_permissions.place.is_none() {
            return;
        }
        let interactive = self.session_permissions.interactive;
        let ask_caller = self.session_permissions.ask_caller;
        self.session_permissions = match self.session_dir.as_deref() {
            Some(dir) => crate::engine::permission::SessionPermissions::load(dir),
            None => Default::default(),
        };
        self.session_permissions.interactive = interactive;
        self.session_permissions.ask_caller = ask_caller;
    }

    /// The place this engine's turn stands in, while one does.
    pub fn turn_place(&self) -> Option<&TurnPlace> {
        self.tools.builtins.turn_place.as_ref()
    }

    /// A place other than the session's own works in its own folder — the
    /// skill's, or the default workspace for home — never in the folder
    /// the session's own place left: its cwd, and the workspace root found
    /// from it. Returns where the session stood; `None` when the place is
    /// the session's own (its folder is the session's).
    fn move_to_place_folder(&mut self, place: &TurnPlace, skill: Option<&Skill>) -> Option<Folder> {
        if place.own {
            return None;
        }
        let folder = match skill {
            Some(s) => s.working_dir(),
            None => crate::util::resolve_path(&crate::paths::default_workspace()),
        };
        let stood = Folder {
            cwd: self.tools.builtins.replace_session_cwd(Some(folder)),
            ws_root: self.cfg.ws_root.clone(),
            tools_root: self.tools.builtins.root().to_path_buf(),
            memory_cwd: self.memory_cwd.clone(),
        };
        self.check_working_folder_change();
        Some(stood)
    }

    /// Stand where the session stood before another place moved it.
    fn back_to_folder(&mut self, folder: Folder) {
        self.tools.builtins.replace_session_cwd(folder.cwd);
        self.cfg.ws_root = folder.ws_root;
        self.tools.builtins.set_workspace_root(folder.tools_root);
        self.memory_cwd = folder.memory_cwd;
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod grants_tests;
#[cfg(test)]
mod loop_tests;

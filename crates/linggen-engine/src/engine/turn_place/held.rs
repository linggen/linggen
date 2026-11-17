//! What a turn takes up from an app — the active skill, the skill dir its
//! tools resolve against, the turn's place — held so that only this module
//! sets them: anyone reads one (`Deref`) or clears it, and the compiler
//! keeps every write here, where a day is asked first
//! (`Tools::kept_out_of_day`).

/// One value only `engine::turn_place` sets.
#[derive(Debug, Clone)]
pub struct Held<T>(Option<T>);

impl<T> Default for Held<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<T> std::ops::Deref for Held<T> {
    type Target = Option<T>;
    fn deref(&self) -> &Option<T> {
        &self.0
    }
}

impl<T> Held<T> {
    /// Nothing held.
    pub(crate) fn clear(&mut self) {
        self.0 = None;
    }

    /// Hold `value`. Only this module may: the setters here ask whether a
    /// day keeps the app out before they call it.
    pub(in crate::engine::turn_place) fn set(&mut self, value: Option<T>) {
        self.0 = value;
    }

    /// Hold `value` in a test's own engine.
    #[cfg(test)]
    pub(crate) fn set_for_test(&mut self, value: Option<T>) {
        self.0 = value;
    }
}

impl crate::engine::AgentEngine {
    /// Take up `skill` as the active skill — with the skill dir its tools
    /// resolve against when `with_tools` — unless a day keeps it out. The
    /// one setter of both (`activate_skill` calls it).
    pub(crate) async fn take_up(&mut self, skill: crate::engine::skill::Skill, with_tools: bool) -> bool {
        if self.tools.builtins.kept_out_of_day(&skill).await {
            return false;
        }
        if with_tools {
            let dir = skill.skill_dir.clone().map(|d| (skill.name.clone(), d));
            self.tools.builtins.active_skill.set(dir);
        }
        self.active_skill.set(Some(skill));
        true
    }

    /// Lend `skill`'s tools to a run that is not its own session (a mission
    /// the skill ships): only the dir its tools resolve against — no
    /// SKILL.md — unless a day keeps it out.
    pub(crate) async fn lend_tools_dir(&mut self, skill: &crate::engine::skill::Skill) -> bool {
        if self.tools.builtins.kept_out_of_day(skill).await {
            return false;
        }
        let dir = skill.skill_dir.clone().map(|d| (skill.name.clone(), d));
        self.tools.builtins.active_skill.set(dir);
        true
    }
}

impl crate::engine::tools::Tools {
    /// A subagent stands where its parent's turn stands.
    pub(crate) fn stand_with_parent(&mut self, place: Option<super::TurnPlace>) {
        self.turn_place.set(place);
    }
}

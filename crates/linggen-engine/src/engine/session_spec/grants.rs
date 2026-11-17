//! A session's preset grants: what it may do without asking, set when it is
//! made — a skill's declared paths, a mission's, an `agent_run` delegate's
//! posture. One function ([`stamp`]) writes them into the session's grant
//! table; every writer goes through it (`lock_tests.rs`).

use super::SessionSpec;
use crate::engine::permission::{
    effective_mode_for_path, Grants, PermissionMode, SessionPermissions,
};
use crate::engine::AgentEngine;
use std::path::Path;

/// The grants a session is made with, in the order they are written: a
/// later path can narrow or replace an earlier one (`set_path_mode`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PresetGrants {
    pub paths: Vec<(String, PermissionMode)>,
    pub covered: Covered,
}

/// What a path already granted at its level gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Covered {
    /// Written again, as every other path (a mission's, a delegate's).
    #[default]
    Written,
    /// Left as it is: the session's table keeps what the person set under
    /// it (a skill taken up in a person's session).
    Kept,
}

impl PresetGrants {
    /// A skill's declared `permission.paths`, taken up in a person's session.
    pub fn of_skill(perm: &Grants) -> Self {
        Self {
            paths: owned(perm),
            covered: Covered::Kept,
        }
    }

    /// A mission's declared paths, then its skill's: the skill's — the ones
    /// its tools were written against — can narrow a broader mission grant.
    pub fn of_mission(mission: Option<&Grants>, skill: Option<&Grants>) -> Self {
        Self {
            paths: [mission, skill]
                .into_iter()
                .flatten()
                .flat_map(owned)
                .collect(),
            covered: Covered::Written,
        }
    }
}

fn owned(perm: &Grants) -> Vec<(String, PermissionMode)> {
    perm.iter_grants()
        .map(|(path, mode)| (path.to_string(), mode))
        .collect()
}

/// Write `grants` into `perms`: whether a path's grant changed.
fn stamp(grants: &PresetGrants, perms: &mut SessionPermissions) -> bool {
    let mut changed = false;
    for (path, mode) in &grants.paths {
        let current = effective_mode_for_path(&perms.path_modes, Path::new(path));
        if grants.covered == Covered::Kept && current == Some(*mode) {
            continue;
        }
        perms.set_path_mode(path, *mode);
        changed = true;
    }
    changed
}

impl SessionSpec {
    /// A session made before any engine runs it (an `agent_run` delegate):
    /// its preset grants on `base`, saved as its `permission.json`.
    pub fn seed(&self, mut base: SessionPermissions, session_dir: &Path) {
        stamp(&self.grants, &mut base);
        base.save(session_dir);
    }
}

impl AgentEngine {
    /// Stamp `grants` on this engine's session permissions (a skill taken
    /// up, a mission entered): whether any changed. The caller saves.
    pub fn stamp_grants(&mut self, grants: &PresetGrants) -> bool {
        stamp(grants, &mut self.session_permissions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::permission::{apply_grants, PathGrant};

    fn grants(rows: &[(&str, &str)]) -> Grants {
        Grants {
            paths: rows
                .iter()
                .map(|(path, mode)| PathGrant {
                    path: path.to_string(),
                    mode: mode.to_string(),
                })
                .collect(),
            warning: None,
        }
    }

    fn table(perms: &SessionPermissions) -> Vec<(String, PermissionMode)> {
        perms
            .path_modes
            .iter()
            .map(|pm| (pm.path.clone(), pm.mode))
            .collect()
    }

    /// Today's skill writer, as it was before the seam.
    fn skill_rule(perm: &Grants, perms: &mut SessionPermissions) -> bool {
        let mut changed = false;
        for (path, mode) in perm.iter_grants() {
            if effective_mode_for_path(&perms.path_modes, Path::new(path)) != Some(mode) {
                perms.set_path_mode(path, mode);
                changed = true;
            }
        }
        changed
    }

    fn start(rows: &[(&str, PermissionMode)]) -> SessionPermissions {
        let mut perms = SessionPermissions::default();
        for (path, mode) in rows {
            perms.set_path_mode(path, *mode);
        }
        perms
    }

    const CASES: &[&[(&str, &str)]] = &[
        &[],
        &[("~/a", "edit"), ("~/a/b", "edit")],
        &[("~/a/b", "edit"), ("~/a", "edit")],
        &[("~/a", "admin"), ("~/a/b", "read"), ("~/a", "read")],
        &[("*", "read"), ("/w", "edit")],
        &[("~/p", "edit")],
        &[("~/p/sub", "admin")],
    ];

    fn starts() -> Vec<SessionPermissions> {
        use PermissionMode::*;
        vec![
            start(&[]),
            start(&[("~/p", Edit), ("~/p/sub", Admin)]),
            start(&[("*", Read)]),
        ]
    }

    #[test]
    fn a_skills_grants_are_written_as_before() {
        for case in CASES {
            let perm = grants(case);
            for before in starts() {
                let (mut old, mut new) = (before.clone(), before);
                let was = skill_rule(&perm, &mut old);
                let now = stamp(&PresetGrants::of_skill(&perm), &mut new);
                assert_eq!((table(&new), now), (table(&old), was), "{case:?}");
            }
        }
    }

    #[test]
    fn a_missions_grants_are_written_as_before() {
        for mission in CASES {
            for skill in CASES {
                let (m, s) = (grants(mission), grants(skill));
                for before in starts() {
                    let (mut old, mut new) = (before.clone(), before);
                    apply_grants(&m, &mut old);
                    apply_grants(&s, &mut old);
                    stamp(&PresetGrants::of_mission(Some(&m), Some(&s)), &mut new);
                    assert_eq!(table(&new), table(&old), "{mission:?} then {skill:?}");
                }
            }
        }
        let none = PresetGrants::of_mission(None, None);
        assert!(none.paths.is_empty());
    }
}

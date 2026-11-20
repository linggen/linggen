//! The shared decision for a read target, independent of the tool that names it.

use super::model::{effective_mode_for_path, is_under_temp, reads_own_dir};
use super::sensitive::{self, Protected};
use super::{PermissionMode, SessionPermissions};
use std::path::Path;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReadOrigin {
    #[default]
    Other,
    LocalOwner,
    LocalDelegate,
    LocalTask,
}

impl ReadOrigin {
    pub(crate) fn task_child(self) -> Self {
        match self {
            Self::Other => Self::Other,
            _ => Self::LocalTask,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReadDecision {
    Allow,
    Ask,
    Refuse,
}

/// An explicitly granted directory in which the session can edit the target.
pub(super) fn in_edit_folder(target: &Path, perms: &SessionPermissions) -> bool {
    if effective_mode_for_path(&perms.path_modes, target) < Some(PermissionMode::Edit) {
        return false;
    }
    let real = sensitive::resolved(target);
    perms.path_modes.iter().filter(|g| g.path != "*").any(|g| {
        let grant = sensitive::resolved(Path::new(&g.path));
        g.mode >= PermissionMode::Edit && grant.is_dir()
            && real.strip_prefix(&grant).ok().is_some_and(|rel| crate::util::inside(&grant, rel).is_some())
    })
}

/// Whether this exact target can be read. A wildcard is never a named
/// grant. The edit exception also requires an actual directory grant.
pub(super) fn decide(
    target: &Path,
    perms: &SessionPermissions,
    origin: ReadOrigin,
    coding: bool,
    under_ceiling: bool,
) -> ReadDecision {
    let real = sensitive::resolved(target);
    let mode = effective_mode_for_path(&perms.path_modes, &real);
    let protection = sensitive::protected(target);
    if let Some(Protected::Secret(file)) = &protection {
        if under_ceiling || origin != ReadOrigin::LocalOwner || !perms.interactive {
            return ReadDecision::Refuse;
        }
        let named = perms.path_modes.iter().filter(|g| g.path != "*").any(|g| {
            let grant = sensitive::resolved(Path::new(&g.path));
            g.mode >= PermissionMode::Read
                && (sensitive::same(&grant, file)
                    || file.parent().is_some_and(|parent| sensitive::same(&grant, parent)))
        });
        return if named && mode >= Some(PermissionMode::Read) { ReadDecision::Allow } else { ReadDecision::Ask };
    }
    if reads_own_dir(perms, PermissionMode::Read, &real) {
        return ReadDecision::Allow;
    }
    if let Some(Protected::Place(root)) = protection {
        let named = perms.path_modes.iter().filter(|g| g.path != "*").any(|g| {
            let grant = sensitive::resolved(Path::new(&g.path));
            (g.mode >= PermissionMode::Read && sensitive::within(&grant, &root)
                && sensitive::within(&real, &grant))
                || (g.mode >= PermissionMode::Edit && grant.is_dir()
                    && real.strip_prefix(&grant).ok().is_some_and(|rel| crate::util::inside(&grant, rel).is_some()))
        });
        return if named && mode >= Some(PermissionMode::Read) { ReadDecision::Allow } else { ReadDecision::Ask };
    }
    if mode >= Some(PermissionMode::Read) || is_under_temp(&real)
        || (coding && origin != ReadOrigin::Other && !under_ceiling && target.canonicalize().is_ok())
    {
        ReadDecision::Allow
    } else {
        ReadDecision::Ask
    }
}

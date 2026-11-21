//! The shared decision for a read target, independent of the tool that names it.

use super::model::{effective_mode_for_path, is_under_temp, reads_own_dir};
use super::sensitive::{self, Protected};
use super::{PermissionMode, SessionPermissions};
use std::path::Path;

/// What a turn stands as for reads — how freely it may look, and whether a
/// secret file may be asked of its person. Only [`ReadStart::standing`] says
/// which turn stands as which; no other production code names a variant
/// (lock: `turn_start::structure_lock::a_standing_for_reads_is_named_in_one_place`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadOrigin {
    Other,
    LocalOwner,
    LocalDelegate,
    LocalTask,
}

/// A turn that was started for no one in particular reads as nothing more
/// than `ReadStart::NoStanding`.
impl Default for ReadOrigin {
    fn default() -> Self {
        ReadStart::NoStanding.standing()
    }
}

/// Who started a turn, as reading sees it: one closed set, every start made
/// anywhere (the server's `TurnStart` projects onto it) and what it may read
/// without asking decided in [`Self::standing`] alone — a new kind of start
/// does not compile until it says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadStart {
    /// The owner typed it, at this Mac, in their own chat: the only start a
    /// secret file may be asked of its owner for.
    OwnerTyped,
    /// A run a host program at this Mac asked for through the owner's own
    /// door (the MCP door, no token): it looks as freely as the owner's
    /// coding chat, and no one is there to ask about a secret.
    HostAtThisMac,
    /// A turn nobody typed (a person's words from a remote surface, a
    /// follow-up, the engine's own notice, an automatic prompt read) in a
    /// session whose last turn stood as the given one: it keeps how freely
    /// that looked, and the owner's say over a secret file does not carry
    /// into it.
    Unattended(ReadOrigin),
    /// A `Task` child of a turn that stood as the given one: the same.
    TaskChild(ReadOrigin),
    /// No standing: a door that is not this Mac's, a mission, a private app's
    /// or a skill's page, a session no start has been put on yet.
    NoStanding,
}

impl ReadStart {
    pub fn standing(self) -> ReadOrigin {
        match self {
            Self::OwnerTyped => ReadOrigin::LocalOwner,
            Self::HostAtThisMac => ReadOrigin::LocalDelegate,
            Self::Unattended(was) | Self::TaskChild(was) => was.looks_as_free_but_untyped(),
            Self::NoStanding => ReadOrigin::Other,
        }
    }
}

impl ReadOrigin {
    /// What a turn nobody typed stands as: the same freedom to look, never
    /// the owner's say over a secret file.
    fn looks_as_free_but_untyped(self) -> Self {
        match self {
            Self::Other => Self::Other,
            Self::LocalOwner | Self::LocalDelegate | Self::LocalTask => Self::LocalTask,
        }
    }

    /// Whether a secret file may be asked of this origin's person: the
    /// owner's own typing only.
    fn may_ask_for_secret(self) -> bool {
        match self {
            Self::LocalOwner => true,
            Self::LocalDelegate | Self::LocalTask | Self::Other => false,
        }
    }

    /// Whether a coding session reads a file it can open without asking.
    fn looks_freely_when_coding(self) -> bool {
        match self {
            Self::Other => false,
            Self::LocalOwner | Self::LocalDelegate | Self::LocalTask => true,
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
    let home = dirs::home_dir().map(|home| sensitive::resolved(&home));
    perms.path_modes.iter().filter(|g| g.path != "*").any(|g| {
        let grant = sensitive::resolved(Path::new(&g.path));
        g.mode >= PermissionMode::Edit && grant.is_dir() && grant.parent().is_some()
            && home.as_ref().is_none_or(|home| !sensitive::within(home, &grant))
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
        if under_ceiling || !origin.may_ask_for_secret() || !perms.interactive {
            return ReadDecision::Refuse;
        }
        let named = perms.path_modes.iter().filter(|g| g.path != "*").any(|g| {
            let grant = sensitive::resolved(Path::new(&g.path));
            g.mode >= PermissionMode::Read
                && sensitive::same(&grant, file)
        });
        return if named && mode >= Some(PermissionMode::Read) { ReadDecision::Allow } else { ReadDecision::Ask };
    }
    if reads_own_dir(perms, PermissionMode::Read, &real) {
        return ReadDecision::Allow;
    }
    if let Some(Protected::Place(root)) = protection {
        let named = perms.path_modes.iter().filter(|g| g.path != "*").any(|g| {
            let grant = sensitive::resolved(Path::new(&g.path));
            g.mode >= PermissionMode::Read && sensitive::within(&grant, &root)
                && sensitive::within(&real, &grant)
        }) || in_edit_folder(&real, perms);
        return if named && mode >= Some(PermissionMode::Read) { ReadDecision::Allow } else { ReadDecision::Ask };
    }
    if mode >= Some(PermissionMode::Read) || is_under_temp(&real)
        || (coding && origin.looks_freely_when_coding() && !under_ceiling && target.canonicalize().is_ok())
    {
        ReadDecision::Allow
    } else {
        ReadDecision::Ask
    }
}

#[cfg(test)]
mod unattended_tests {
    use super::{ReadOrigin, ReadStart};

    /// A turn nobody typed never stands as the owner; a looker stays a looker.
    #[test]
    fn a_turn_nobody_typed_is_never_the_owners() {
        let untyped = |was| ReadStart::Unattended(was).standing();
        assert_eq!(untyped(ReadOrigin::LocalOwner), ReadOrigin::LocalTask);
        assert_eq!(untyped(ReadOrigin::LocalDelegate), ReadOrigin::LocalTask);
        assert_eq!(untyped(ReadOrigin::LocalTask), ReadOrigin::LocalTask);
        assert_eq!(untyped(ReadOrigin::Other), ReadOrigin::Other);
    }

    /// Every start, as the table says: the design table, executable.
    #[test]
    fn every_start_stands_as_the_table_says() {
        use ReadOrigin::{LocalDelegate, LocalOwner, LocalTask, Other};
        assert_eq!(ReadStart::OwnerTyped.standing(), LocalOwner);
        assert_eq!(ReadStart::HostAtThisMac.standing(), LocalDelegate);
        assert_eq!(ReadStart::NoStanding.standing(), Other);
        assert_eq!(ReadOrigin::default(), Other);
        for was in [LocalOwner, LocalDelegate, LocalTask, Other] {
            assert_eq!(ReadStart::TaskChild(was).standing(), ReadStart::Unattended(was).standing(), "{was:?}");
        }
    }
}

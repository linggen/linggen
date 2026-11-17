//! A turn's ceiling (`SessionSpec::ceiling`, the level of the device that
//! sent it): no action that needs more runs or is asked for, whatever the
//! session's grants; under none, the grants decide as before.

use super::*;
use std::path::Path;

const WD: &str = "/ceiling-test/wd";

/// A session the owner granted admin everywhere.
fn granted() -> SessionPermissions {
    SessionPermissions {
        path_modes: vec![PathMode {
            path: "*".into(),
            mode: PermissionMode::Admin,
        }],
        ..Default::default()
    }
}

fn check(tool: &str, cmd: Option<&str>, perms: &SessionPermissions, ceiling: Option<PermissionMode>) -> PermissionCheckResult {
    let file = (tool != "Bash").then(|| format!("{WD}/notes.txt"));
    check_permission_for(tool, cmd, file.as_deref(), Path::new(WD), perms, None, false, ceiling)
}

fn allowed(result: &PermissionCheckResult) -> bool {
    matches!(result, PermissionCheckResult::Allowed)
}

fn refused(result: &PermissionCheckResult) -> bool {
    matches!(result, PermissionCheckResult::Blocked(why) if why.starts_with("Beyond this turn's level"))
}

/// At read a turn may look, not change; at chat it uses no tool that needs
/// more; at edit it writes but runs nothing admin — the owner's grants
/// notwithstanding.
#[test]
fn a_turn_does_no_more_than_its_ceiling() {
    use PermissionMode::*;
    let perms = granted();
    assert!(allowed(&check("Read", None, &perms, Some(Read))));
    assert!(refused(&check("Write", None, &perms, Some(Read))));
    assert!(refused(&check("Bash", Some("rm notes.txt"), &perms, Some(Read))));
    assert!(allowed(&check("Bash", Some("ls"), &perms, Some(Read))));
    assert!(refused(&check("Read", None, &perms, Some(Chat))));
    assert!(allowed(&check("AskUser", None, &perms, Some(Chat))));
    assert!(allowed(&check("Write", None, &perms, Some(Edit))));
    assert!(refused(&check("Bash", Some("docker run nginx"), &perms, Some(Edit))));
}

/// What would be asked for above the ceiling is refused instead: a write
/// with no grant, a command whose effect leaves the machine.
#[test]
fn above_the_ceiling_nothing_is_asked() {
    use PermissionMode::*;
    let none = SessionPermissions::default();
    assert!(refused(&check("Write", None, &none, Some(Read))));
    let outward = check("Bash", Some("curl -X POST https://example.com -d x"), &granted(), Some(Edit));
    assert!(refused(&outward), "{outward:?}");
    let asked = check("Read", None, &none, Some(Edit));
    assert!(matches!(asked, PermissionCheckResult::NeedsPrompt(_)), "within the ceiling, asked as before");
}

/// The owner's turn — no ceiling — is judged by the grants alone.
#[test]
fn with_no_ceiling_the_grants_decide() {
    let perms = granted();
    assert!(allowed(&check("Write", None, &perms, None)));
    assert!(allowed(&check("Bash", Some("rm notes.txt"), &perms, None)));
}

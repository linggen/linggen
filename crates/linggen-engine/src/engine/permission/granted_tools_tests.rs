//! A granted build tool in the Bash check (`permission-spec.md` § Granted
//! tools): a program the tables do not know is judged at edit, not admin,
//! where the session may edit — and nothing else moves.

use super::toolchains::{save, OwnerTools, Tool};
use super::*;
use std::path::{Path, PathBuf};

/// A fake Mac: a bin folder with build tools, the owner's list, and a
/// project folder outside the OS temp roots (those are never gated).
struct Mac {
    _home: tempfile::TempDir,
    bin: PathBuf,
    list: PathBuf,
}

const WD: &str = "/granted-tools-test/wd";

impl Mac {
    fn new(listed: &[&str]) -> Self {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        for name in ["flutter", "git", "python3", "mystery"] {
            let path = bin.join(name);
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        let list = home.path().join("toolchains.toml");
        let mac = Self {
            _home: home,
            bin,
            list,
        };
        mac.list_only(listed);
        mac
    }

    /// The owner's list holds exactly `names`, each at its file in `bin`.
    fn list_only(&self, names: &[&str]) {
        let tools = names
            .iter()
            .map(|name| Tool {
                name: name.to_string(),
                path: self.bin.join(name),
            })
            .collect();
        save(
            &self.list,
            &OwnerTools {
                seeded: None,
                tools,
            },
        )
        .unwrap();
    }

    fn tool(&self, name: &str) -> String {
        self.bin.join(name).to_string_lossy().to_string()
    }

    fn check(&self, cmd: &str, perms: &SessionPermissions) -> PermissionCheckResult {
        let search = self.bin.to_string_lossy().to_string();
        let source = ToolSource {
            file: &self.list,
            search: &search,
        };
        check_permission_with("Bash", Some(cmd), None, Path::new(WD), perms, None, &source)
    }
}

fn session(cwd_mode: PermissionMode, granted: &[&str]) -> SessionPermissions {
    let mut perms = SessionPermissions {
        granted_tools: granted.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    perms.set_path_mode("*", PermissionMode::Read);
    perms.set_path_mode(WD, cwd_mode);
    perms
}

fn allowed(result: &PermissionCheckResult) -> bool {
    matches!(result, PermissionCheckResult::Allowed)
}

fn asks_at(result: &PermissionCheckResult) -> Option<PermissionMode> {
    match result {
        PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { target_mode, .. }) => {
            Some(*target_mode)
        }
        _ => None,
    }
}

#[test]
fn a_granted_unknown_program_runs_where_the_session_may_edit() {
    let mac = Mac::new(&["flutter"]);
    let perms = session(PermissionMode::Edit, &["flutter"]);
    for cmd in [
        "flutter test".to_string(),
        format!("{} test", mac.tool("flutter")),
        format!("cd {WD} && flutter build ios"),
        "flutter test > out.log 2>&1".to_string(),
    ] {
        assert!(allowed(&mac.check(&cmd, &perms)), "{cmd}");
    }
}

#[test]
fn the_same_program_not_granted_still_asks_for_admin() {
    let mac = Mac::new(&["flutter"]);
    for perms in [
        session(PermissionMode::Edit, &[]),
        // Granted by name, but the owner's list does not hold it.
        session(PermissionMode::Edit, &["mystery"]),
    ] {
        assert_eq!(
            asks_at(&mac.check("flutter test", &perms)),
            Some(PermissionMode::Admin)
        );
        assert_eq!(
            asks_at(&mac.check("mystery run", &perms)),
            Some(PermissionMode::Admin)
        );
    }
    // A word of the granted name that runs another file is not the tool.
    let perms = session(PermissionMode::Edit, &["flutter"]);
    let other = tempfile::tempdir().unwrap();
    let fake = other.path().join("flutter");
    std::fs::write(&fake, "").unwrap();
    let cmd = format!("{} test", fake.display());
    assert_eq!(
        asks_at(&mac.check(&cmd, &perms)),
        Some(PermissionMode::Admin)
    );
    // Nor is a word naming the tool by a relative path.
    assert_eq!(
        asks_at(&mac.check("./bin/flutter test", &perms)),
        Some(PermissionMode::Admin)
    );
}

#[test]
fn a_granted_tool_still_needs_edit_where_it_runs() {
    let mac = Mac::new(&["flutter"]);
    let perms = session(PermissionMode::Read, &["flutter"]);
    for cmd in [
        "flutter test".to_string(),
        // Its own file is read, and the run is still judged where it runs.
        format!("{} test", mac.tool("flutter")),
        // A scratch path is free; the run is not.
        "flutter create /tmp/x".to_string(),
    ] {
        assert_eq!(
            asks_at(&mac.check(&cmd, &perms)),
            Some(PermissionMode::Edit),
            "{cmd}"
        );
    }
}

#[test]
fn a_known_program_keeps_its_tables_when_granted() {
    let mac = Mac::new(&["git", "python3"]);
    let perms = session(PermissionMode::Edit, &["git", "python3"]);
    for cmd in [
        "git -c x=y status".to_string(),
        format!("{} -c x=y status", mac.tool("git")),
        format!("{} status", mac.tool("git")),
        "git config user.name x".to_string(),
        "python3 x.py".to_string(),
    ] {
        assert_eq!(
            asks_at(&mac.check(&cmd, &perms)),
            Some(PermissionMode::Admin),
            "{cmd}"
        );
    }
    assert!(allowed(&mac.check("git status", &perms)));
}

#[test]
fn only_the_granted_segment_is_lowered() {
    let mac = Mac::new(&["flutter"]);
    let perms = session(PermissionMode::Edit, &["flutter"]);
    for cmd in [
        "flutter test && mystery",
        "flutter test $FLAGS",
        "flutter test <(cat x)",
        "env FOO=1 flutter test",
    ] {
        assert_eq!(
            asks_at(&mac.check(cmd, &perms)),
            Some(PermissionMode::Admin),
            "{cmd}"
        );
    }
}

#[test]
fn the_deny_floor_holds_for_a_granted_program() {
    let mac = Mac::new(&["flutter"]);
    let perms = session(PermissionMode::Admin, &["flutter"]);
    for cmd in [
        "flutter test && sudo rm -rf /x".to_string(),
        format!("{} test; sudo ls", mac.tool("flutter")),
        "rm -rf / && flutter test".to_string(),
    ] {
        assert!(
            matches!(mac.check(&cmd, &perms), PermissionCheckResult::Blocked(_)),
            "{cmd}"
        );
    }
}

#[test]
fn an_owners_removal_takes_effect_at_the_next_check() {
    let mac = Mac::new(&["flutter"]);
    let perms = session(PermissionMode::Edit, &["flutter"]);
    assert!(allowed(&mac.check("flutter test", &perms)));
    mac.list_only(&[]);
    assert_eq!(
        asks_at(&mac.check("flutter test", &perms)),
        Some(PermissionMode::Admin)
    );
    // A missing list grants nothing.
    std::fs::remove_file(&mac.list).unwrap();
    assert_eq!(
        asks_at(&mac.check("flutter test", &perms)),
        Some(PermissionMode::Admin)
    );
}

#[test]
fn granted_tools_are_kept_and_an_empty_set_is_not_written() {
    let dir = tempfile::tempdir().unwrap();
    SessionPermissions::default().save(dir.path());
    let text = std::fs::read_to_string(dir.path().join("permission.json")).unwrap();
    assert!(!text.contains("granted_tools"), "{text}");
    session(PermissionMode::Edit, &["flutter"]).save(dir.path());
    assert_eq!(
        SessionPermissions::load(dir.path()).granted_tools,
        ["flutter"]
    );
}

#[test]
fn a_task_child_inherits_its_parents_granted_tools() {
    let mac = Mac::new(&["flutter"]);
    let parent = session(PermissionMode::Edit, &["flutter"]);
    let mut child = SessionPermissions::default();
    crate::engine::tools::inherit_granted_tools(&mut child, &parent.granted_tools);
    child.path_modes = parent.path_modes.clone();
    assert!(allowed(&mac.check("flutter test", &child)));
}

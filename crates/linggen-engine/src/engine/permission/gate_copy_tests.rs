use super::*;
use std::path::Path;

fn check(cmd: &str, mode: PermissionMode, wildcard: bool) -> PermissionCheckResult {
    let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut perms = SessionPermissions::default();
    if wildcard {
        perms.set_path_mode("*", PermissionMode::Read);
    }
    perms.set_path_mode(&cwd.to_string_lossy(), mode);
    check_permission("Bash", Some(cmd), None, cwd, &perms, None)
}

#[test]
fn gate_new_cases() {
    let w = env!("CARGO_MANIFEST_DIR");
    let cases = [
        (
            format!("cd {w}/src && /opt/tools/bin/flutter test"),
            PermissionMode::Admin,
        ),
        ("/opt/tools/bin/flutter test".into(), PermissionMode::Admin),
        (
            format!("cp /outside/a.toml {w}/a.toml"),
            PermissionMode::Edit,
        ),
        (
            format!("cp /outside/a /outside/b {w}/"),
            PermissionMode::Edit,
        ),
        (
            format!("ln -s /outside/target {w}/link"),
            PermissionMode::Edit,
        ),
        (
            format!("rsync -a /outside/src/ {w}/dst/"),
            PermissionMode::Edit,
        ),
    ];
    let failed: Vec<_> = cases
        .iter()
        .filter_map(|(cmd, mode)| {
            let r = check(cmd, *mode, true);
            (!matches!(r, PermissionCheckResult::Allowed)).then(|| format!("{cmd}: {r:?}"))
        })
        .collect();
    assert!(failed.is_empty(), "{failed:#?}");
}

#[test]
fn gate_still_cases() {
    let w = env!("CARGO_MANIFEST_DIR");
    let cases = [
        (
            "/opt/tools/bin/flutter test".into(),
            PermissionMode::Edit,
            true,
        ),
        (
            "/opt/tools/bin/flutter test".into(),
            PermissionMode::Admin,
            false,
        ),
        (
            "/opt/tools/bin/flutter create /outside/app".into(),
            PermissionMode::Admin,
            true,
        ),
        (
            "/opt/tools/bin/tool > /outside/out.txt".into(),
            PermissionMode::Admin,
            true,
        ),
        (format!("cp {w}/a /outside/b"), PermissionMode::Edit, true),
        (
            "cp /outside/a /outside/b".into(),
            PermissionMode::Edit,
            true,
        ),
        (format!("mv /outside/a {w}/a"), PermissionMode::Edit, true),
        (
            format!("rsync -a --remove-source-files /outside/src/ {w}/dst/"),
            PermissionMode::Edit,
            true,
        ),
        (format!("cp -t {w} /outside/a"), PermissionMode::Edit, true),
        ("rm /outside/a".into(), PermissionMode::Edit, true),
        ("touch /outside/a".into(), PermissionMode::Edit, true),
        ("mkdir /outside/d".into(), PermissionMode::Edit, true),
        (format!("cp /outside/a {w}/a"), PermissionMode::Read, true),
        (
            format!("rsync --unknown /outside/a {w}/a"),
            PermissionMode::Edit,
            true,
        ),
        (
            format!("cp /outside/a {w}/a > /outside/out"),
            PermissionMode::Edit,
            true,
        ),
    ];
    for (cmd, mode, wildcard) in cases {
        let r = check(&cmd, mode, wildcard);
        assert!(
            matches!(r, PermissionCheckResult::NeedsPrompt(_)),
            "{cmd}: {r:?}"
        );
    }
    for cmd in ["/usr/bin/sudo ls", "/bin/rm -rf /"] {
        assert!(
            matches!(
                check(cmd, PermissionMode::Admin, true),
                PermissionCheckResult::Blocked(_)
            ),
            "{cmd}"
        );
    }
}

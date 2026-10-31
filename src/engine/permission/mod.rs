//! Permission system — see `doc/permission-spec.md`.
//!
//! State per session: a list of (path, mode) grants. Effective mode for the
//! current cwd is the most-specific matching grant, or `chat` (no tools) if
//! nothing covers it. Reads are gated like writes — there is no "reads are
//! free" carveout. A short hardcoded deny floor blocks the worst foot-guns
//! regardless of mode. No config rule layer.
//!
//! Layers
//! ------
//! - [`classify`] — bash command classification (which tier a command needs).
//! - [`model`]  — types, path matching, tier mapping,
//!   and `check_permission` (the core decision function).
//! - [`store`]  — `SessionPermissions` (load/save permission.json, set_path_mode).
//! - [`prompt`] — AskUser widget construction and answer parsing.

mod classify;
mod manifest;
mod model;
mod prompt;
mod scripts;
mod store;

pub use manifest::{apply_grants, Grants, PathGrant};
pub use model::{
    check_permission, effective_mode_for_path, parse_skill_tier, PathMode, PermissionAction,
    PermissionCheckResult, PermissionMode, PromptKind,
};
pub use prompt::{
    build_exceeds_ceiling_question, parse_exceeds_ceiling_answer, permission_target_summary,
};
pub use store::SessionPermissions;

#[cfg(test)]
mod tests {
    use super::classify::classify_bash_command;
    use super::model::{extract_command_paths, is_hardcoded_deny, tool_action_tier, BashClass};
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn test_permission_mode_ordering() {
        assert!(PermissionMode::Chat < PermissionMode::Read);
        assert!(PermissionMode::Read < PermissionMode::Edit);
        assert!(PermissionMode::Edit < PermissionMode::Admin);
    }

    #[test]
    fn test_permission_mode_serde() {
        let json = serde_json::to_string(&PermissionMode::Edit).unwrap();
        assert_eq!(json, "\"edit\"");
        let mode: PermissionMode = serde_json::from_str("\"admin\"").unwrap();
        assert_eq!(mode, PermissionMode::Admin);
    }

    #[test]
    fn test_session_permissions_serde_roundtrip() {
        let mut sp = SessionPermissions::default();
        sp.path_modes.push(PathMode {
            path: "~/workspace/linggen".to_string(),
            mode: PermissionMode::Edit,
        });
        let json = serde_json::to_string_pretty(&sp).unwrap();
        let loaded: SessionPermissions = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.path_modes.len(), 1);
        assert_eq!(loaded.path_modes[0].mode, PermissionMode::Edit);
        assert!(loaded.interactive);
    }

    #[test]
    fn test_session_permissions_load_legacy_fields_ignored() {
        let tmp = std::env::temp_dir().join("linggen_perm_legacy_test");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let legacy = r#"{
            "path_modes": [{"path": "~/x", "mode": "edit"}],
            "locked": true,
            "policy": {"on_exceed": "ask", "on_ask_rule": "ask"},
            "allows": ["Bash(git push *)"],
            "denied_sigs": ["Bash:rm -rf dist"]
        }"#;
        fs::write(tmp.join("permission.json"), legacy).unwrap();
        let loaded = SessionPermissions::load(&tmp);
        assert_eq!(loaded.path_modes.len(), 1);
        assert_eq!(loaded.path_modes[0].path, "~/x");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_set_path_mode_prunes_children() {
        let mut sp = SessionPermissions::default();
        sp.set_path_mode("/tmp/project/src/a.rs", PermissionMode::Edit);
        sp.set_path_mode("/tmp/project/src/b.rs", PermissionMode::Edit);
        sp.set_path_mode("/tmp/other", PermissionMode::Read);
        assert_eq!(sp.path_modes.len(), 3);

        sp.set_path_mode("/tmp/project", PermissionMode::Edit);
        assert_eq!(sp.path_modes.len(), 2);
        assert!(sp.path_modes.iter().any(|pm| pm.path == "/tmp/project"));
        assert!(sp.path_modes.iter().any(|pm| pm.path == "/tmp/other"));
    }

    #[test]
    fn test_set_path_mode_prunes_tilde_children() {
        let mut sp = SessionPermissions::default();
        sp.set_path_mode("~/workspace/linggen/src/main.rs", PermissionMode::Edit);
        sp.set_path_mode("~/workspace/linggen", PermissionMode::Edit);
        assert_eq!(sp.path_modes.len(), 1);
        assert_eq!(sp.path_modes[0].path, "~/workspace/linggen");
    }

    #[test]
    fn wildcard_matches_every_path_but_concrete_grants_are_more_specific() {
        for modes in [
            vec![
                PathMode {
                    path: "*".into(),
                    mode: PermissionMode::Admin,
                },
                PathMode {
                    path: "/wildcard-test/private".into(),
                    mode: PermissionMode::Read,
                },
            ],
            vec![
                PathMode {
                    path: "/wildcard-test/private".into(),
                    mode: PermissionMode::Read,
                },
                PathMode {
                    path: "*".into(),
                    mode: PermissionMode::Admin,
                },
            ],
        ] {
            assert_eq!(
                effective_mode_for_path(&modes, Path::new("/unrelated/file")),
                Some(PermissionMode::Admin)
            );
            assert_eq!(
                effective_mode_for_path(&modes, Path::new("/wildcard-test/private/file")),
                Some(PermissionMode::Read)
            );
            assert_eq!(
                effective_mode_for_path(&modes, Path::new("/wildcard-test/private-other")),
                Some(PermissionMode::Admin)
            );
        }
    }

    #[test]
    fn setting_wildcard_replaces_concrete_grants_and_can_be_overridden() {
        let mut sp = SessionPermissions::default();
        sp.set_path_mode("/wildcard-test/a", PermissionMode::Admin);
        sp.set_path_mode("/wildcard-test/b", PermissionMode::Read);
        sp.set_path_mode("*", PermissionMode::Edit);
        assert_eq!(sp.path_modes.len(), 1);
        assert_eq!(sp.path_modes[0].path, "*");
        assert_eq!(sp.path_modes[0].mode, PermissionMode::Edit);

        sp.set_path_mode("/wildcard-test/a", PermissionMode::Read);
        assert_eq!(
            effective_mode_for_path(&sp.path_modes, Path::new("/wildcard-test/a/file")),
            Some(PermissionMode::Read)
        );
        assert_eq!(
            effective_mode_for_path(&sp.path_modes, Path::new("/wildcard-test/b/file")),
            Some(PermissionMode::Edit)
        );
        sp.set_path_mode("*", PermissionMode::Chat);
        assert_eq!(sp.path_modes.len(), 1);
        assert_eq!(sp.path_modes[0].mode, PermissionMode::Chat);
    }

    #[test]
    fn regression_bash_mutations_are_not_read() {
        let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut sp = SessionPermissions::default();
        sp.set_path_mode("*", PermissionMode::Read);
        for cmd in [
            "echo hello > out.txt && true",
            "echo \"$(touch out.txt)\"",
            "git branch example",
            "find . -delete",
            "docker system prune",
            "ls > /dev/null-output",
        ] {
            let result = check_permission("Bash", Some(cmd), None, cwd, &sp, None);
            assert!(matches!(result, PermissionCheckResult::NeedsPrompt(_)), "{cmd}: {result:?}");
        }
    }

    #[test]
    fn regression_quoted_pipe_is_not_a_command_separator() {
        let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut sp = SessionPermissions::default();
        sp.set_path_mode("*", PermissionMode::Read);
        let result = check_permission("Bash", Some("echo 'a|b'"), None, cwd, &sp, None);
        assert!(matches!(result, PermissionCheckResult::Allowed), "{result:?}");
    }

    #[test]
    fn regression_copy_from_temp_checks_relative_output() {
        let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut sp = SessionPermissions::default();
        sp.set_path_mode(&cwd.to_string_lossy(), PermissionMode::Read);
        for cmd in ["cp /tmp/source ./out", "cp /tmp/source out"] {
            let result = check_permission("Bash", Some(cmd), None, cwd, &sp, None);
            assert!(matches!(result, PermissionCheckResult::NeedsPrompt(_)), "{cmd}: {result:?}");
        }
    }

    #[test]
    fn bash_classification_regressions() {
        for (cmd, expected) in [
            ("echo 'a|b'", BashClass::Read),
            ("echo '>'", BashClass::Read),
            ("ls && echo hi > out", BashClass::Write),
            ("echo $(rm x)", BashClass::Admin),
            ("echo `touch x`", BashClass::Admin),
            ("ls > /dev/null-output", BashClass::Write),
            ("ls 2>/dev/null", BashClass::Read),
            ("git tag v1", BashClass::Write),
            ("git stash drop", BashClass::Write),
            ("find . -exec rm {} +", BashClass::Admin),
            ("docker system prune", BashClass::Admin),
            ("docker system df", BashClass::Read),
            (r"echo \' ; rm -rf x ; echo \'", BashClass::Admin),
            (r"echo a\;rm -rf x", BashClass::Read),
            ("cat <(rm -rf x)", BashClass::Admin),
            ("tee >(rm -rf x)", BashClass::Admin),
            ("echo '<(x)'", BashClass::Read),
            ("env rm -rf x", BashClass::Admin),
            ("env -i FOO=1 ls", BashClass::Read),
            ("env", BashClass::Read),
            ("env > out", BashClass::Write),
            ("echo hi >| out", BashClass::Write),
            ("git tag -l", BashClass::Read),
            ("git remote get-url origin", BashClass::Read),
        ] {
            assert_eq!(classify_bash_command(cmd), expected, "{cmd}");
        }
    }

    #[test]
    fn quoted_flags_are_judged_dequoted() {
        for (cmd, expected) in [
            (r"find . '-exec' rm -rf {} +", BashClass::Admin),
            (r"find . -ex''ec rm {} \;", BashClass::Admin),
            (r"find . \-delete", BashClass::Admin),
            (r#"find . "-execdir" ls \;"#, BashClass::Admin),
            ("find . -fprint0 out", BashClass::Write),
            ("find . -fprint out", BashClass::Write),
            ("find . -name '*.rs'", BashClass::Read),
            (r"\rm x", BashClass::Admin),
            ("'git' 'push'", BashClass::Write),
            ("g''it status", BashClass::Read),
            (r"git branch '-D' x", BashClass::Write),
            (r#"echo "a\"b""#, BashClass::Read),
        ] {
            assert_eq!(classify_bash_command(cmd), expected, "{cmd}");
        }
    }

    #[test]
    fn env_flags_never_read_below_their_reach() {
        for (cmd, expected) in [
            ("env -S'rm -rf x'", BashClass::Admin),
            ("env -S 'rm -rf x'", BashClass::Admin),
            ("env --split-string='rm -rf x'", BashClass::Admin),
            ("env --split-string 'ls'", BashClass::Admin),
            ("env -C /etc ls", BashClass::Admin),
            ("env -P /bin ls", BashClass::Admin),
            ("env --frobnicate ls", BashClass::Admin),
            ("env -u HOME ls", BashClass::Read),
            ("env -uHOME --unset=PATH ls", BashClass::Read),
            ("env -i FOO=1 git push", BashClass::Write),
            ("env -i", BashClass::Read),
        ] {
            assert_eq!(classify_bash_command(cmd), expected, "{cmd}");
        }
    }

    #[test]
    fn read_programs_that_write_or_execute() {
        use BashClass::{Admin, Read, Write};
        for (cmd, expected) in [
            ("fd -x rm", Admin),
            ("fd -X rm", Admin),
            ("fd -Hx rm", Admin),
            ("fd --exec rm", Admin),
            ("fd --exec-batch rm", Admin),
            ("fd -e rs", Read),
            ("rg --pre ./run x", Admin),
            ("rg --pre=./run x", Admin),
            ("rg --pre-glob '*.gz' x", Read),
            ("bat --pager 'rm -rf x' f", Admin),
            ("bat --paging=never f", Read),
            ("git diff --output=x", Write),
            ("git log --output x", Write),
            ("git show --outp=x HEAD", Write),
            ("git log --oneline", Read),
            ("sort -o out in", Write),
            ("sort -nro out in", Write),
            ("sort --output=out in", Write),
            ("sort --o=out in", Write),
            ("sort --compress-program=sh in", Admin),
            ("sort -n in", Read),
            ("uniq in out", Write),
            ("uniq -c in", Read),
            ("uniq -f 1 in", Read),
            ("tree -o out", Write),
            ("tree -L 2", Read),
            ("yq -i '.a = 1' f.yaml", Write),
            ("yq '.a' f.yaml", Read),
            ("go env -w GOFLAGS=x", Write),
            ("go env GOPATH", Read),
            ("go vet -vettool=/bin/sh ./...", Admin),
            ("npm audit fix", Write),
            ("pnpm audit --fix", Write),
            ("npm audit", Read),
            ("npx ls", Write),
            ("cargo clippy --fix", Write),
            ("cargo clippy", Read),
            ("curl -I -o out https://x", Admin),
            ("curl -I https://x -O", Admin),
            ("curl --head --remote-name https://x", Admin),
            ("curl -sSI https://x", Read),
            ("curl -I -X DELETE https://x", Admin),
            ("curl -H 'X-I: 1' https://x", Admin),
            ("curl https://x", Admin),
            ("wget --spider https://x", Read),
            ("wget --spider -O out https://x", Admin),
            ("sed -n 's/a/b/p' f", Write),
            ("sed 's/a/b/e' f", Admin),
            ("sed '1e rm -rf x' f", Admin),
            ("sed -e 'p' -e 'e ls' f", Admin),
            ("sed -ne 's/x/y/e' f", Admin),
            ("sed --expression='e ls' f", Admin),
            ("sed --exp 'e ls' f", Admin),
            ("sed -f script.sed f", Admin),
            ("sed -i '' 's/a/b/' f", Write),
            ("sed -i '' 's/a/b/e' f", Admin),
            ("sed '/x/{s/a/b/;p}' f", Write),
            ("sed '/exec/d' f", Write),
            ("sed 's|a|b|g;y/ab/cd/' f", Write),
            ("awk '{print $1}' f", Write),
            ("awk '{system(\"rm x\")}' f", Admin),
            ("awk '{print | \"sh\"}' f", Admin),
            ("awk 'BEGIN{\"date\" | getline d}'", Admin),
            ("awk -F, '/a|b/ {print}' f", Write),
            ("awk '$1 == \"a|b\" || $2 {print}' f", Write),
            ("awk -f prog.awk f", Admin),
            ("awk -v x=1 '{print x}' f", Write),
        ] {
            assert_eq!(classify_bash_command(cmd), expected, "{cmd}");
        }
    }

    #[test]
    fn deny_floor_sees_through_quotes() {
        assert!(is_hardcoded_deny("'sudo' ls"));
        assert!(is_hardcoded_deny(r"s\udo ls"));
        assert!(is_hardcoded_deny("rm -rf '/'"));
    }

    #[test]
    fn test_classify_bash_read() {
        assert_eq!(classify_bash_command("ls"), BashClass::Read);
        assert_eq!(classify_bash_command("git status"), BashClass::Read);
        assert_eq!(classify_bash_command("cargo check"), BashClass::Read);
        assert_eq!(classify_bash_command("python --version"), BashClass::Read);
    }

    #[test]
    fn test_classify_bash_write() {
        assert_eq!(classify_bash_command("mkdir -p src/new"), BashClass::Write);
        assert_eq!(
            classify_bash_command("git push origin main"),
            BashClass::Write
        );
        assert_eq!(classify_bash_command("cargo build"), BashClass::Write);
    }

    #[test]
    fn test_classify_bash_admin() {
        assert_eq!(classify_bash_command("rm -rf dist"), BashClass::Admin);
        assert_eq!(classify_bash_command("docker run nginx"), BashClass::Admin);
    }

    #[test]
    fn test_classify_bash_macshifu_read() {
        // apple-shifu's command vocabulary should classify as read so the skill
        // can run with mode: read instead of mode: admin on /.
        assert_eq!(classify_bash_command("sw_vers"), BashClass::Read);
        assert_eq!(classify_bash_command("vm_stat"), BashClass::Read);
        assert_eq!(classify_bash_command("uptime"), BashClass::Read);
        assert_eq!(classify_bash_command("netstat -an"), BashClass::Read);
        assert_eq!(classify_bash_command("ifconfig en0"), BashClass::Read);
        assert_eq!(classify_bash_command("sysctl -n hw.ncpu"), BashClass::Read);
        assert_eq!(classify_bash_command("spctl --status"), BashClass::Read);
        assert_eq!(classify_bash_command("csrutil status"), BashClass::Read);
        assert_eq!(classify_bash_command("fdesetup status"), BashClass::Read);
        assert_eq!(classify_bash_command("brew list"), BashClass::Read);
        assert_eq!(classify_bash_command("brew --version"), BashClass::Read);
        assert_eq!(classify_bash_command("docker images"), BashClass::Read);
        assert_eq!(classify_bash_command("docker system df"), BashClass::Read);
        assert_eq!(classify_bash_command("docker ps"), BashClass::Read);
        // Mutations stay admin.
        assert_eq!(classify_bash_command("brew install foo"), BashClass::Admin);
        assert_eq!(classify_bash_command("docker run nginx"), BashClass::Admin);
        assert_eq!(
            classify_bash_command("docker rm container"),
            BashClass::Admin
        );
    }

    #[test]
    fn test_classify_bash_compound() {
        assert_eq!(classify_bash_command("ls && rm foo"), BashClass::Admin);
        assert_eq!(classify_bash_command("ls | grep foo"), BashClass::Read);
        assert_eq!(classify_bash_command("mkdir d && cp a b"), BashClass::Write);
    }

    #[test]
    fn test_classify_bash_redirect() {
        assert_eq!(classify_bash_command("echo hi > out.txt"), BashClass::Write);
        assert_eq!(classify_bash_command("ls > files.txt"), BashClass::Write);
        assert_eq!(
            classify_bash_command("du -sh ~/Desktop 2>/dev/null"),
            BashClass::Read
        );
    }

    #[test]
    fn test_hardcoded_deny_sudo() {
        assert!(is_hardcoded_deny("sudo apt install foo"));
        assert!(is_hardcoded_deny("sudo -i"));
        assert!(is_hardcoded_deny("sudoedit /etc/hosts"));
        assert!(is_hardcoded_deny("ls && sudo rm /etc/foo"));
    }

    #[test]
    fn test_hardcoded_deny_rm_rf_root() {
        assert!(is_hardcoded_deny("rm -rf /"));
        assert!(is_hardcoded_deny("rm -rf /*"));
        assert!(is_hardcoded_deny("rm -fr /"));
        assert!(!is_hardcoded_deny("rm -rf dist"));
        assert!(!is_hardcoded_deny("rm -rf ./build"));
    }

    #[test]
    fn test_hardcoded_deny_dd_blockdev() {
        assert!(is_hardcoded_deny("dd if=/dev/zero of=/dev/sda"));
        assert!(is_hardcoded_deny("dd of=/dev/disk2 if=foo.iso"));
        assert!(is_hardcoded_deny("dd if=foo of=/dev/nvme0n1"));
        assert!(!is_hardcoded_deny(
            "dd if=/dev/zero of=/tmp/blob bs=1M count=10"
        ));
    }

    #[test]
    fn test_hardcoded_deny_mkfs() {
        assert!(is_hardcoded_deny("mkfs /dev/sda1"));
        assert!(is_hardcoded_deny("mkfs.ext4 /dev/sda1"));
        assert!(is_hardcoded_deny("mkfs.btrfs /dev/sdb"));
    }

    #[test]
    fn test_hardcoded_deny_forkbomb() {
        assert!(is_hardcoded_deny(":(){:|:&};:"));
        assert!(is_hardcoded_deny(":() { :|:& };:"));
    }

    #[test]
    fn test_hardcoded_deny_chown_chmod_root() {
        assert!(is_hardcoded_deny("chown -R nobody /"));
        assert!(is_hardcoded_deny("chmod -R 777 /"));
        assert!(!is_hardcoded_deny("chown user:user /etc/hosts"));
    }

    #[test]
    fn test_hardcoded_deny_safe_commands() {
        assert!(!is_hardcoded_deny("ls"));
        assert!(!is_hardcoded_deny("cargo build"));
        assert!(!is_hardcoded_deny("git push origin main"));
        assert!(!is_hardcoded_deny(""));
    }

    #[test]
    fn test_effective_mode_for_path_basic() {
        let modes = vec![
            PathMode {
                path: "~/workspace/linggen".to_string(),
                mode: PermissionMode::Edit,
            },
            PathMode {
                path: "~/workspace/other".to_string(),
                mode: PermissionMode::Read,
            },
        ];

        if let Some(home) = dirs::home_dir() {
            assert_eq!(
                effective_mode_for_path(&modes, &home.join("workspace/linggen/src/main.rs")),
                Some(PermissionMode::Edit),
            );
            assert_eq!(
                effective_mode_for_path(&modes, &home.join("workspace/other/README.md")),
                Some(PermissionMode::Read),
            );
            assert_eq!(
                effective_mode_for_path(&modes, &home.join("Documents/notes.txt")),
                None,
            );
        }
    }

    #[test]
    fn test_effective_mode_no_zone_default() {
        // Critical: /tmp no longer auto-grants edit.
        let modes: Vec<PathMode> = vec![];
        assert_eq!(effective_mode_for_path(&modes, Path::new("/tmp/x")), None);
        assert_eq!(
            effective_mode_for_path(&modes, Path::new("/var/tmp/x")),
            None
        );
    }

    #[test]
    fn test_effective_mode_most_specific_wins() {
        let modes = vec![
            PathMode {
                path: "~/workspace".to_string(),
                mode: PermissionMode::Read,
            },
            PathMode {
                path: "~/workspace/linggen".to_string(),
                mode: PermissionMode::Admin,
            },
        ];
        if let Some(home) = dirs::home_dir() {
            assert_eq!(
                effective_mode_for_path(&modes, &home.join("workspace/linggen/x")),
                Some(PermissionMode::Admin),
            );
            assert_eq!(
                effective_mode_for_path(&modes, &home.join("workspace/other/y")),
                Some(PermissionMode::Read),
            );
        }
    }

    #[test]
    fn test_chat_mode_returns_needs_prompt_regardless_of_interactive() {
        // Contract: `check_permission` is mode-driven and does NOT consult
        // `session_permissions.interactive`. Chat mode always returns
        // NeedsPrompt(ExceedsCeiling) for any concrete tool. The interactive
        // distinction lives in tool_exec.rs:
        //   - interactive=true  (owner)            → prompt fires, user opts in
        //   - interactive=false (mission/consumer) → silent block, no prompt
        let mut sp = SessionPermissions::default();
        sp.set_path_mode("~/workspace", PermissionMode::Chat);

        if let Some(home) = dirs::home_dir() {
            let cwd = home.join("workspace");
            let target = home.join("workspace/file.txt");
            let target_s = target.to_str().unwrap();

            sp.interactive = true;
            let r1 = check_permission("Read", None, Some(target_s), &cwd, &sp, None);
            assert!(
                matches!(
                    r1,
                    PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { .. })
                ),
                "owner case: expected NeedsPrompt, got {r1:?}",
            );

            sp.interactive = false;
            let r2 = check_permission("Read", None, Some(target_s), &cwd, &sp, None);
            assert!(
                matches!(
                    r2,
                    PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { .. })
                ),
                "non-interactive case: expected NeedsPrompt, got {r2:?}",
            );
        }
    }

    #[test]
    fn test_session_permissions_default_is_interactive() {
        let sp = SessionPermissions::default();
        assert!(sp.interactive);
    }

    #[test]
    fn test_check_permission_within_ceiling() {
        if let Some(home) = dirs::home_dir() {
            let cwd = home.join("workspace/linggen");
            let mut sp = SessionPermissions::default();
            sp.set_path_mode("~/workspace/linggen", PermissionMode::Edit);

            let result = check_permission(
                "Read",
                None,
                Some(cwd.join("src/main.rs").to_str().unwrap()),
                &cwd,
                &sp,
                None,
            );
            assert!(matches!(result, PermissionCheckResult::Allowed));

            let result = check_permission(
                "Write",
                None,
                Some(cwd.join("src/main.rs").to_str().unwrap()),
                &cwd,
                &sp,
                None,
            );
            assert!(matches!(result, PermissionCheckResult::Allowed));
        }
    }

    #[test]
    fn test_check_permission_exceeds_ceiling() {
        if let Some(home) = dirs::home_dir() {
            let cwd = home.join("workspace/linggen");
            let mut sp = SessionPermissions::default();
            sp.set_path_mode("~/workspace/linggen", PermissionMode::Read);

            let result = check_permission(
                "Write",
                None,
                Some(cwd.join("src/main.rs").to_str().unwrap()),
                &cwd,
                &sp,
                None,
            );
            assert!(matches!(
                result,
                PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { .. })
            ));
        }
    }

    #[test]
    fn test_check_permission_no_grant_outside_cwd_prompts() {
        if let Some(home) = dirs::home_dir() {
            let cwd = home.join("workspace/linggen");
            let mut sp = SessionPermissions::default();
            sp.set_path_mode("~/workspace/linggen", PermissionMode::Edit);

            // Read /etc/hosts — outside any grant, must prompt.
            let result = check_permission("Read", None, Some("/etc/hosts"), &cwd, &sp, None);
            assert!(matches!(
                result,
                PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { .. })
            ));
        }
    }

    #[test]
    fn test_check_permission_bash_args_gate_by_arg_path_not_cwd() {
        // The headline bug this gate fixes: a session with read on /A and cwd
        // /A could previously run `bash ls /B` because the gate only checked
        // cwd's tier. Now the path arg is checked — if /B isn't covered, the
        // call prompts to upgrade /B.
        if let Some(home) = dirs::home_dir() {
            let cwd = home.join("a");
            let other = home.join("b/foo");
            let other_str = other.to_string_lossy().to_string();
            let mut sp = SessionPermissions::default();
            sp.set_path_mode(&cwd.to_string_lossy(), PermissionMode::Read);

            // Use a non-temp path: temp dirs are always-allowed scratch, so
            // they wouldn't exercise the arg-path gate this test covers.
            let result = check_permission(
                "Bash",
                Some(&format!("ls {other_str}")),
                None,
                &cwd,
                &sp,
                None,
            );
            assert!(
                matches!(
                    result,
                    PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { .. })
                ),
                "ls {other_str} from cwd {} should prompt — arg path not covered. got {result:?}",
                cwd.display(),
            );

            sp.set_path_mode(&home.join("b").to_string_lossy(), PermissionMode::Read);
            let result = check_permission(
                "Bash",
                Some(&format!("ls {other_str}")),
                None,
                &cwd,
                &sp,
                None,
            );
            assert!(
                matches!(result, PermissionCheckResult::Allowed),
                "ls {other_str} with read on ~/b should be allowed. got {result:?}"
            );
        }
    }

    #[test]
    fn test_check_permission_bash_no_args_uses_cwd() {
        // Bash without absolute path args (e.g. `cargo build`, `ls .`) operates
        // in cwd — the cwd's tier is what gates it.
        if let Some(home) = dirs::home_dir() {
            let cwd = home.join("project");
            let mut sp = SessionPermissions::default();
            sp.set_path_mode(&cwd.to_string_lossy(), PermissionMode::Edit);

            let result = check_permission("Bash", Some("cargo build"), None, &cwd, &sp, None);
            assert!(
                matches!(result, PermissionCheckResult::Allowed),
                "cargo build in edit-mode cwd should be allowed. got {result:?}"
            );
        }
    }

    #[test]
    fn test_check_permission_bash_arg_already_granted_passes() {
        // Skill activation grants admin on ~/.linggen/skills. A bash command
        // that only references that granted path (e.g. `bash ~/.linggen/skills/foo/run.sh`)
        // should pass without re-prompting, even if cwd has a lower tier.
        if let Some(home) = dirs::home_dir() {
            let cwd = home.join("project");
            let mut sp = SessionPermissions::default();
            sp.set_path_mode(&cwd.to_string_lossy(), PermissionMode::Read);
            sp.set_path_mode("~/.linggen/skills", PermissionMode::Admin);

            let result = check_permission(
                "Bash",
                Some("bash ~/.linggen/skills/foo/run.sh"),
                None,
                &cwd,
                &sp,
                None,
            );
            assert!(matches!(result, PermissionCheckResult::Allowed),
                "bash on a granted path should be allowed even if cwd is lower-tier. got {result:?}");
        }
    }

    #[test]
    fn test_extract_command_paths_catches_upward_relative() {
        // Relative args that climb out of cwd via `..` are extracted so they
        // get gated against their resolved target — this closes the
        // `cat ../../.ssh/id_rsa` escape. Relative args without `..` stay
        // inside cwd and are omitted (already covered by the cwd-tier check).
        assert_eq!(
            extract_command_paths("cat ../../.ssh/id_rsa"),
            vec!["../../.ssh/id_rsa".to_string()],
        );
        assert_eq!(
            extract_command_paths("cp a/../../b/x ./out"),
            vec!["a/../../b/x".to_string()],
        );
        // No `..` component → not extracted.
        assert!(extract_command_paths("cargo build --release").is_empty());
        assert!(extract_command_paths("cat ./local.txt").is_empty());
        // A git range (`main..feature`) is one component, not a `..` traversal.
        assert!(extract_command_paths("git log main..feature").is_empty());
    }

    #[test]
    fn test_check_permission_bash_relative_escape_gated() {
        // End-to-end: a `..` arg that leaves the granted cwd must prompt, not
        // pass at cwd's tier. Uses the real home dir as cwd so `..` resolves
        // on disk (normalize_path collapses it against an existing directory).
        if let Some(home) = dirs::home_dir() {
            let mut sp = SessionPermissions::default();
            sp.set_path_mode(&home.to_string_lossy(), PermissionMode::Read);

            let escape = check_permission(
                "Bash",
                Some("cat ../other-xyz/id_rsa"),
                None,
                &home,
                &sp,
                None,
            );
            assert!(
                matches!(escape, PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { .. })),
                "cat ../other-xyz/id_rsa must prompt — it climbs out of the granted cwd. got {escape:?}",
            );

            if let Some(parent) = home.parent() {
                sp.set_path_mode(&parent.to_string_lossy(), PermissionMode::Read);
                let granted = check_permission(
                    "Bash",
                    Some("cat ../other-xyz/id_rsa"),
                    None,
                    &home,
                    &sp,
                    None,
                );
                assert!(
                    matches!(granted, PermissionCheckResult::Allowed),
                    "with read on the parent, the escaped read should pass. got {granted:?}"
                );
            }
        }
    }

    #[test]
    fn test_extract_command_paths_ignores_jq_alternative_operator() {
        // jq's `//` alternative operator must NOT be read as a filesystem
        // path — otherwise the dream's TTL command demands admin on `//`.
        let cmd = "ling-mem list --episodic --older-than \"7d\" --format json \
                   | jq -r '.data.episodic_ttl_days // env.X // 7'";
        let paths = extract_command_paths(cmd);
        assert!(
            paths.is_empty(),
            "jq // should not be extracted; got {paths:?}"
        );

        // Real paths still come through; bare-slash junk is dropped.
        let mixed = extract_command_paths("cat ~/.linggen/x.json / // ///");
        assert_eq!(mixed, vec!["~/.linggen/x.json".to_string()]);

        // Path-like words INSIDE a quoted string arg (a search query) are
        // NOT path args — the quoted span stays one word.
        let q = extract_command_paths(
            "ling-mem search \"binary path /usr/local/bin ~/.local/bin state ~/.linggen\" --limit 8",
        );
        assert!(
            q.is_empty(),
            "quoted search query must not yield path args; got {q:?}"
        );

        // But a genuinely quoted path ARG still counts.
        assert_eq!(
            extract_command_paths("cat \"/etc/hosts\""),
            vec!["/etc/hosts".to_string()],
        );
    }

    #[test]
    fn test_dream_commands_allowed_with_linggen_admin_grant() {
        // End-to-end repro of the click-a-day → dream flow. The skill
        // grants admin on ~/.linggen (SKILL.md permission.paths) and runs
        // from cwd ~/.linggen. Both the scan.sh exec and the Phase 3 TTL
        // command (which contains jq's `//` operator) must run WITHOUT a
        // permission prompt.
        let Some(home) = dirs::home_dir() else { return };
        let cwd = home.join(".linggen");
        let mut sp = SessionPermissions::default();
        sp.set_path_mode(&cwd.to_string_lossy(), PermissionMode::Admin);

        let scan = "bash ~/.linggen/skills/shared-memory/scripts/scan.sh 2026-05-29";
        assert!(
            matches!(
                check_permission("Bash", Some(scan), None, &cwd, &sp, None),
                PermissionCheckResult::Allowed
            ),
            "scan.sh under admin ~/.linggen should be allowed",
        );

        let ttl = "set -e; TTL_DAYS=$(curl -s http://127.0.0.1:9528/api/config 2>/dev/null \
                   | jq -r '.data.episodic_ttl_days // env.LING_MEM_EPISODIC_TTL_DAYS // 7'); \
                   ling-mem list --episodic --older-than \"${TTL_DAYS}d\" --format json \
                   | jq -c 'del(.vector)'";
        assert!(
            matches!(
                check_permission("Bash", Some(ttl), None, &cwd, &sp, None),
                PermissionCheckResult::Allowed
            ),
            "Phase 3 TTL command (with jq //) should be allowed, not re-prompt for admin on //",
        );
    }

    #[test]
    fn test_temp_dir_is_always_allowed_scratch() {
        let cwd = dirs::home_dir().unwrap_or_default().join(".linggen");
        let sp = SessionPermissions::default(); // no grants at all

        // A Bash write to /tmp (admin-class — unknown programs) is allowed
        // with NO grant: temp is always-available scratch.
        let cmd = "tail -n +2 ~/.linggen/memory/.scan-output.jsonl | jq -r '.x' \
                   > /tmp/dream_users.txt && wc -l /tmp/dream_users.txt";
        // (~/.linggen still needs its own grant; grant it so only /tmp is at issue.)
        let mut sp2 = SessionPermissions::default();
        sp2.set_path_mode(
            &dirs::home_dir().unwrap().join(".linggen").to_string_lossy(),
            PermissionMode::Admin,
        );
        assert!(
            matches!(
                check_permission("Bash", Some(cmd), None, &cwd, &sp2, None),
                PermissionCheckResult::Allowed
            ),
            "admin-class bash writing to /tmp should be allowed as scratch",
        );

        // A Write tool straight to a /tmp file is allowed with no grant.
        assert!(matches!(
            check_permission("Write", None, Some("/tmp/scratch.json"), &cwd, &sp, None),
            PermissionCheckResult::Allowed
        ));

        // Deny floor still wins even when the target is in temp.
        assert!(matches!(
            check_permission("Bash", Some("sudo rm -rf /tmp/x"), None, &cwd, &sp, None),
            PermissionCheckResult::Blocked(_)
        ));
    }

    #[test]
    fn test_check_permission_hardcoded_deny_overrides_admin() {
        let mut sp = SessionPermissions::default();
        sp.set_path_mode("~", PermissionMode::Admin);
        let cwd = dirs::home_dir().unwrap_or_default();

        let result = check_permission("Bash", Some("sudo rm -rf /"), None, &cwd, &sp, None);
        assert!(matches!(result, PermissionCheckResult::Blocked(_)));
    }

    /// The safety property phase 1 of the MCP client rests on: a tool from a
    /// server the USER added is gated, and gated *by default* — not by a rule
    /// someone has to remember to write for each one. The memory server's
    /// ungated pass is precisely what it must never inherit, and it doesn't
    /// come from the tier: every MCP tool sits at Admin here, and only a
    /// server's own `gated: false` waves its tools through.
    #[test]
    fn mcp_tools_are_admin_tier_never_chat() {
        assert_eq!(
            tool_action_tier("mcp__github__create_issue"),
            PermissionMode::Admin
        );
        assert_eq!(
            tool_action_tier("mcp__ling-mem__memory_search"),
            PermissionMode::Admin
        );
        // The contrast that makes the point.
        assert_eq!(tool_action_tier("AskUser"), PermissionMode::Chat);
    }

    #[test]
    fn test_tool_action_tier() {
        assert_eq!(tool_action_tier("Read"), PermissionMode::Read);
        assert_eq!(tool_action_tier("WebFetch"), PermissionMode::Read);
        assert_eq!(tool_action_tier("Write"), PermissionMode::Edit);
        assert_eq!(tool_action_tier("Bash"), PermissionMode::Admin);
    }

    /// Memory used to be two built-in tools pinned at Chat. It is ling-mem's
    /// own MCP server now, and nothing here knows its name: the static tier of
    /// any MCP tool is Admin, and memory's exemption comes from its server's
    /// declared `gated: false`, checked in `check_permission`. Re-introducing a
    /// built-in `Memory_*` would put a second memory surface in front of the
    /// model — this is the test that notices.
    #[test]
    fn memory_is_no_longer_a_built_in_tool() {
        assert!(crate::engine::tools::builtin_tier("Memory_query").is_none());
        assert!(crate::engine::tools::builtin_tier("Memory_write").is_none());
        assert_eq!(tool_action_tier("Memory_query"), PermissionMode::Admin);
    }

    #[test]
    fn test_parse_skill_tier() {
        assert_eq!(parse_skill_tier("read"), Some(PermissionMode::Read));
        assert_eq!(parse_skill_tier("edit"), Some(PermissionMode::Edit));
        assert_eq!(parse_skill_tier("admin"), Some(PermissionMode::Admin));
        assert_eq!(parse_skill_tier("nonsense"), None);
    }

    #[test]
    fn test_build_exceeds_ceiling_question() {
        let dirs = vec!["~/repo".to_string(), "~/repo/a".to_string(), "~/repo/a/b".to_string()];
        let labels = |mode: PermissionMode| -> Vec<String> {
            build_exceeds_ceiling_question("x", &mode, "~/repo/a/b/aaa.rs", &dirs)
                .options
                .into_iter()
                .map(|o| o.label)
                .collect()
        };
        // Below admin: each directory, the level everywhere, admin everywhere, Deny.
        assert_eq!(
            labels(PermissionMode::Read),
            [
                "grant:at:read:~/repo",
                "grant:at:read:~/repo/a",
                "grant:at:read:~/repo/a/b",
                "grant:all:read",
                "grant:all:admin",
                "Deny"
            ]
        );
        // Admin request: the directories, admin everywhere, Deny.
        assert_eq!(
            labels(PermissionMode::Admin),
            [
                "grant:at:admin:~/repo",
                "grant:at:admin:~/repo/a",
                "grant:at:admin:~/repo/a/b",
                "grant:all:admin",
                "Deny"
            ]
        );
        let q = build_exceeds_ceiling_question(
            "Edit src/main.rs",
            &PermissionMode::Edit,
            "~/repo/a/b/aaa.rs",
            &dirs,
        );
        let metadata = q.permission.unwrap();
        assert_eq!(metadata.path, "~/repo/a/b/aaa.rs");
        assert_eq!(metadata.mode, PermissionMode::Edit);
    }

    #[test]
    fn test_parse_exceeds_ceiling_answer() {
        use crate::engine::tools::AskUserAnswer;
        let answer = |index, selections: Vec<&str>| AskUserAnswer {
            question_index: index,
            selected: selections.into_iter().map(str::to_owned).collect(),
            custom_text: None,
        };
        let owned = |labels: &[&str]| labels.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let admin = PermissionMode::Admin;
        let admin_offer = owned(&[
            "grant:at:admin:~/repo",
            "grant:at:admin:~/repo/a",
            "grant:all:admin",
            "Deny",
        ]);
        assert_eq!(
            parse_exceeds_ceiling_answer(
                &[answer(0, vec!["grant:at:admin:~/repo/a"])],
                &admin,
                &admin_offer
            ),
            PermissionAction::AllowDirectory {
                all: false,
                mode: PermissionMode::Admin,
                path: "~/repo/a".to_string(),
            }
        );
        assert_eq!(
            parse_exceeds_ceiling_answer(&[answer(0, vec!["grant:all:admin"])], &admin, &admin_offer),
            PermissionAction::AllowDirectory {
                all: true,
                mode: PermissionMode::Admin,
                path: "*".to_string(),
            }
        );
        assert_eq!(
            parse_exceeds_ceiling_answer(&[answer(0, vec!["Deny"])], &admin, &admin_offer),
            PermissionAction::Deny
        );
        // A path that was not on the card cannot be granted.
        assert_eq!(
            parse_exceeds_ceiling_answer(
                &[answer(0, vec!["grant:at:admin:/"])],
                &admin,
                &admin_offer
            ),
            PermissionAction::Deny
        );
        let read = PermissionMode::Read;
        let read_offer = owned(&[
            "grant:at:read:~/repo",
            "grant:all:read",
            "grant:all:admin",
            "Deny",
        ]);
        assert_eq!(
            parse_exceeds_ceiling_answer(&[answer(0, vec!["grant:all:admin"])], &read, &read_offer),
            PermissionAction::AllowDirectory {
                all: true,
                mode: PermissionMode::Admin,
                path: "*".to_string(),
            }
        );
        assert_eq!(
            parse_exceeds_ceiling_answer(
                &[answer(0, vec!["grant:at:read:~/repo"])],
                &read,
                &read_offer
            ),
            PermissionAction::AllowDirectory {
                all: false,
                mode: PermissionMode::Read,
                path: "~/repo".to_string(),
            }
        );
        assert_eq!(
            parse_exceeds_ceiling_answer(
                &[answer(0, vec!["grant:at:admin:~/repo"])],
                &read,
                &read_offer
            ),
            PermissionAction::Deny
        );
        for label in [
            "grant:directory:edit",
            "grant:all:read",
            "grant:all:root",
            "Allow once",
            "Switch this folder to admin",
        ] {
            assert_eq!(
                parse_exceeds_ceiling_answer(&[answer(0, vec![label])], &admin, &admin_offer),
                PermissionAction::Deny,
                "{label} must deny"
            );
        }
        assert_eq!(
            parse_exceeds_ceiling_answer(
                &[answer(0, vec!["grant:all:admin", "Deny"])],
                &admin,
                &admin_offer
            ),
            PermissionAction::Deny
        );
        assert_eq!(
            parse_exceeds_ceiling_answer(&[answer(1, vec!["grant:all:admin"])], &admin, &admin_offer),
            PermissionAction::Deny
        );
        assert_eq!(
            parse_exceeds_ceiling_answer(&[], &admin, &admin_offer),
            PermissionAction::Deny
        );
    }

    #[test]
    fn test_permission_target_summary_bash() {
        let args = serde_json::json!({ "cmd": "cargo build" });
        assert_eq!(
            permission_target_summary("Bash", &args, Path::new("/tmp")),
            "cargo build"
        );
    }

    #[test]
    fn directory_buttons_run_from_the_repo_root_to_the_file() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("perm-ancestors-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("repo");
        std::fs::create_dir_all(repo.join("a/b/c")).unwrap();
        std::fs::create_dir(repo.join(".git")).unwrap();
        let file = repo.join("a/b/c/aaa.rs");
        std::fs::write(&file, "x").unwrap();
        let sp = SessionPermissions::default();
        let result = check_permission(
            "Write",
            None,
            Some(file.to_str().unwrap()),
            &repo,
            &sp,
            None,
        );
        match result {
            PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling {
                directories,
                path,
                ..
            }) => {
                assert!(path.ends_with("aaa.rs"), "{path}");
                assert!(directories[0].ends_with("/repo"), "{}", directories[0]);
                assert!(directories.iter().any(|dir| dir.ends_with("/repo/a")));
                assert!(directories.last().unwrap().ends_with("/c"), "{directories:?}");
            }
            other => panic!("expected a prompt, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A granted workspace `ws/` with `ws/skills/` inside it and a sibling
    /// `other/` beside it — under `target/`, since the OS temp dir is exempt
    /// from the gate and would test nothing.
    fn granted_workspace(name: &str) -> (std::path::PathBuf, SessionPermissions) {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("perm-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("ws/skills")).unwrap();
        std::fs::create_dir_all(root.join("other")).unwrap();
        let ws = root.join("ws").canonicalize().unwrap();
        let mut sp = SessionPermissions::default();
        sp.set_path_mode(&ws.to_string_lossy(), PermissionMode::Admin);
        (ws, sp)
    }

    fn bash(cmd: &str, cwd: &std::path::Path, sp: &SessionPermissions) -> PermissionCheckResult {
        check_permission("Bash", Some(cmd), None, cwd, sp, None)
    }

    fn prompted_path(r: PermissionCheckResult) -> String {
        match r {
            PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { path, .. }) => path,
            other => panic!("expected a prompt, got {other:?}"),
        }
    }

    #[test]
    fn an_upward_path_after_cd_is_resolved_from_where_the_shell_stands() {
        let (ws, sp) = granted_workspace("after-cd");
        let cmd = format!(
            "cd {}/skills 2>/dev/null && git status; ls -d ../skills-wt-* | head",
            ws.display()
        );
        assert!(matches!(
            bash(&cmd, &ws, &sp),
            PermissionCheckResult::Allowed
        ));
        // `cd` is followed through `;` and a flag too.
        let cmd = format!("cd -P {}/skills; cat ../README", ws.display());
        assert!(matches!(
            bash(&cmd, &ws, &sp),
            PermissionCheckResult::Allowed
        ));
    }

    #[test]
    fn a_path_that_really_leaves_the_grant_still_asks_and_offers_a_grant_that_covers_it() {
        let (ws, sp) = granted_workspace("leaves");
        let outside = ws.parent().unwrap().join("other");
        // No `cd`: `..` is the session folder's parent.
        let path = prompted_path(bash("ls ../other", &ws, &sp));
        assert_ne!(
            shellexpand_home(&path),
            ws,
            "offered the grant already held"
        );
        assert!(
            outside.starts_with(shellexpand_home(&path)),
            "{path} does not cover {outside:?}"
        );
        // After a `cd`, two levels up still leaves it.
        let cmd = format!("cd {}/skills && ls ../../other", ws.display());
        let path = prompted_path(bash(&cmd, &ws, &sp));
        assert_ne!(shellexpand_home(&path), ws);
    }

    #[test]
    fn a_cd_that_cannot_be_followed_resolves_from_the_session_folder() {
        let (ws, sp) = granted_workspace("unfollowed");
        // A missing directory, a variable, a subshell: not followed.
        for cmd in [
            format!("cd {}/nope; ls ../../other", ws.display()),
            "cd $SOME_DIR && ls ../other".to_string(),
            format!("(cd {}/skills; ls ../other)", ws.display()),
        ] {
            assert!(
                matches!(bash(&cmd, &ws, &sp), PermissionCheckResult::NeedsPrompt(_)),
                "{cmd}"
            );
        }
    }

    #[test]
    fn the_dotdot_escape_stays_closed_after_a_cd() {
        let (ws, sp) = granted_workspace("escape");
        // A cd into scratch space does not make a `..` out of it free.
        let r = bash("cd /tmp && cat ../etc/hosts", &ws, &sp);
        assert!(matches!(r, PermissionCheckResult::NeedsPrompt(_)), "{r:?}");
        // Walking up out of the grant with `cd ..` and then further.
        let cmd = format!("cd {} && cd .. && ls ../..", ws.display());
        let r = bash(&cmd, &ws, &sp);
        assert!(matches!(r, PermissionCheckResult::NeedsPrompt(_)), "{r:?}");
        // A cd that cannot be resolved (a quoted variable, a bare variable)
        // is not trusted: the paths after it still ask.
        for cmd in [
            r#"cd "$HOME/x" && cat ../../etc/hosts"#,
            "cd $SOME_DIR && cat ../../etc/hosts",
            "cd ~root && cat ../etc/hosts",
        ] {
            let r = bash(cmd, &ws, &sp);
            assert!(
                matches!(r, PermissionCheckResult::NeedsPrompt(_)),
                "{cmd}: {r:?}"
            );
        }
    }

    #[test]
    fn a_cd_is_not_followed_past_a_pipe_an_or_or_a_background() {
        let (ws, sp) = granted_workspace("joints");
        // Session folder is `ws`; `skills/..` would be `ws`, but the shell
        // never moved, so `..` really is ws's parent — outside the grant.
        for cmd in [
            "cd skills | cat ../other/secret",
            "cd skills|cat ../other/secret",
            "cd skills || ls ../other",
            "cd skills||ls ../other",
            "cd skills & ls ../other",
            "cd skills& ls ../other",
            "cd skills &ls ../other",
            "cat x | cd skills && ls ../other",
            "false && cd skills; ls ../other",
            "cd; ls ../other",
        ] {
            let r = bash(cmd, &ws, &sp);
            assert!(
                matches!(r, PermissionCheckResult::NeedsPrompt(_)),
                "{cmd}: {r:?}"
            );
        }
    }

    #[test]
    fn a_cd_is_followed_across_and_semicolon_and_newline() {
        let (ws, sp) = granted_workspace("seq");
        for cmd in [
            "cd skills && ls ../other",
            "cd skills&&ls ../other",
            "cd skills; ls ../other",
            "cd skills;ls ../other",
            "cd skills\nls ../other",
            "cd skills 2>&1 && ls ../other",
            "cd skills && ls ../other | head",
        ] {
            // `../other` from `ws/skills` is `ws/other`, inside the grant.
            let r = bash(cmd, &ws, &sp);
            assert!(matches!(r, PermissionCheckResult::Allowed), "{cmd}: {r:?}");
        }
    }

    fn shellexpand_home(p: &str) -> std::path::PathBuf {
        match p.strip_prefix("~/") {
            Some(rest) => dirs::home_dir().unwrap().join(rest),
            None => std::path::PathBuf::from(p),
        }
    }
}

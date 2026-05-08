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
//! - [`model`]  — types, bash classification, path matching, tier mapping,
//!                and `check_permission` (the core decision function).
//! - [`store`]  — `SessionPermissions` (load/save permission.json, set_path_mode).
//! - [`prompt`] — AskUser widget construction and answer parsing.

mod model;
mod prompt;
mod store;

pub use model::{
    check_permission, effective_mode_for_path, parse_skill_tier, PathMode, PermissionAction,
    PermissionCheckResult, PermissionMode, PromptKind,
};
pub use prompt::{build_exceeds_ceiling_question, permission_target_summary};
pub use store::SessionPermissions;

#[cfg(test)]
mod tests {
    use super::model::{
        classify_bash_command, is_hardcoded_deny, tool_action_tier, BashClass,
    };
    use super::prompt::parse_exceeds_ceiling_answer;
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
    fn test_classify_bash_read() {
        assert_eq!(classify_bash_command("ls"), BashClass::Read);
        assert_eq!(classify_bash_command("git status"), BashClass::Read);
        assert_eq!(classify_bash_command("cargo check"), BashClass::Read);
        assert_eq!(classify_bash_command("python --version"), BashClass::Read);
    }

    #[test]
    fn test_classify_bash_write() {
        assert_eq!(classify_bash_command("mkdir -p src/new"), BashClass::Write);
        assert_eq!(classify_bash_command("git push origin main"), BashClass::Write);
        assert_eq!(classify_bash_command("cargo build"), BashClass::Write);
    }

    #[test]
    fn test_classify_bash_admin() {
        assert_eq!(classify_bash_command("rm -rf dist"), BashClass::Admin);
        assert_eq!(classify_bash_command("docker run nginx"), BashClass::Admin);
    }

    #[test]
    fn test_classify_bash_sysdoctor_read() {
        // sys-doctor's command vocabulary should classify as read so the skill
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
        assert_eq!(classify_bash_command("docker rm container"), BashClass::Admin);
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
        assert_eq!(classify_bash_command("du -sh ~/Desktop 2>/dev/null"), BashClass::Read);
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
        assert!(!is_hardcoded_deny("dd if=/dev/zero of=/tmp/blob bs=1M count=10"));
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
            PathMode { path: "~/workspace/linggen".to_string(), mode: PermissionMode::Edit },
            PathMode { path: "~/workspace/other".to_string(), mode: PermissionMode::Read },
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
        assert_eq!(effective_mode_for_path(&modes, Path::new("/var/tmp/x")), None);
    }

    #[test]
    fn test_effective_mode_most_specific_wins() {
        let modes = vec![
            PathMode { path: "~/workspace".to_string(), mode: PermissionMode::Read },
            PathMode { path: "~/workspace/linggen".to_string(), mode: PermissionMode::Admin },
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
                matches!(r1, PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { .. })),
                "owner case: expected NeedsPrompt, got {r1:?}",
            );

            sp.interactive = false;
            let r2 = check_permission("Read", None, Some(target_s), &cwd, &sp, None);
            assert!(
                matches!(r2, PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { .. })),
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
                "Read", None, Some(cwd.join("src/main.rs").to_str().unwrap()),
                &cwd, &sp, None,
            );
            assert!(matches!(result, PermissionCheckResult::Allowed));

            let result = check_permission(
                "Write", None, Some(cwd.join("src/main.rs").to_str().unwrap()),
                &cwd, &sp, None,
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
                "Write", None, Some(cwd.join("src/main.rs").to_str().unwrap()),
                &cwd, &sp, None,
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
            let result = check_permission(
                "Read", None, Some("/etc/hosts"),
                &cwd, &sp, None,
            );
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
            let mut sp = SessionPermissions::default();
            sp.set_path_mode(&cwd.to_string_lossy(), PermissionMode::Read);

            let result = check_permission(
                "Bash", Some("ls /tmp/foo"), None,
                &cwd, &sp, None,
            );
            assert!(
                matches!(result, PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { .. })),
                "ls /tmp/foo from cwd {} should prompt — /tmp/foo not covered. got {result:?}",
                cwd.display(),
            );

            sp.set_path_mode("/tmp", PermissionMode::Read);
            let result = check_permission(
                "Bash", Some("ls /tmp/foo"), None,
                &cwd, &sp, None,
            );
            assert!(matches!(result, PermissionCheckResult::Allowed),
                "ls /tmp/foo with read on /tmp should be allowed. got {result:?}");
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

            let result = check_permission(
                "Bash", Some("cargo build"), None,
                &cwd, &sp, None,
            );
            assert!(matches!(result, PermissionCheckResult::Allowed),
                "cargo build in edit-mode cwd should be allowed. got {result:?}");
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
                "Bash", Some("bash ~/.linggen/skills/foo/run.sh"), None,
                &cwd, &sp, None,
            );
            assert!(matches!(result, PermissionCheckResult::Allowed),
                "bash on a granted path should be allowed even if cwd is lower-tier. got {result:?}");
        }
    }

    #[test]
    fn test_check_permission_hardcoded_deny_overrides_admin() {
        let mut sp = SessionPermissions::default();
        sp.set_path_mode("~", PermissionMode::Admin);
        let cwd = dirs::home_dir().unwrap_or_default();

        let result = check_permission(
            "Bash", Some("sudo rm -rf /"), None,
            &cwd, &sp, None,
        );
        assert!(matches!(result, PermissionCheckResult::Blocked(_)));
    }

    #[test]
    fn test_tool_action_tier() {
        assert_eq!(tool_action_tier("Read"), PermissionMode::Read);
        assert_eq!(tool_action_tier("WebFetch"), PermissionMode::Read);
        assert_eq!(tool_action_tier("Write"), PermissionMode::Edit);
        assert_eq!(tool_action_tier("Bash"), PermissionMode::Admin);
    }

    #[test]
    fn test_capability_tool_tier_comes_from_registry() {
        assert_eq!(tool_action_tier("Memory_query"), PermissionMode::Read);
        assert_eq!(tool_action_tier("Memory_write"), PermissionMode::Edit);
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
        let q = build_exceeds_ceiling_question("Edit src/main.rs", &PermissionMode::Edit, "~/work");
        assert_eq!(q.options.len(), 3);
        assert_eq!(q.options[0].label, "Switch this folder to edit");
        assert_eq!(q.options[1].label, "Allow once");
        assert_eq!(q.options[2].label, "Deny");
    }

    #[test]
    fn test_parse_exceeds_ceiling_answer() {
        let mode = PermissionMode::Edit;
        assert_eq!(
            parse_exceeds_ceiling_answer("Switch this folder to edit", &mode),
            PermissionAction::AllowSession,
        );
        assert_eq!(
            parse_exceeds_ceiling_answer("Allow once", &mode),
            PermissionAction::AllowOnce,
        );
        assert_eq!(parse_exceeds_ceiling_answer("Deny", &mode), PermissionAction::Deny);
    }

    #[test]
    fn test_permission_target_summary_bash() {
        let args = serde_json::json!({ "cmd": "cargo build" });
        assert_eq!(permission_target_summary("Bash", &args, Path::new("/tmp")), "cargo build");
    }
}

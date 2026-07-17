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

mod manifest;
mod model;
mod prompt;
mod store;

pub use manifest::{apply_grants, parse_mode_str, Grants, PathGrant};
pub use model::{
    check_permission, effective_mode_for_path, parse_skill_tier, PathMode, PermissionAction,
    PermissionCheckResult, PermissionMode, PromptKind,
};
pub use prompt::{build_exceeds_ceiling_question, permission_target_summary};
pub use store::SessionPermissions;

#[cfg(test)]
mod tests {
    use super::model::{
        classify_bash_command, extract_command_paths, is_hardcoded_deny, tool_action_tier,
        BashClass,
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
    fn test_classify_bash_macshifu_read() {
        // mac-shifu's command vocabulary should classify as read so the skill
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
            let other = home.join("b/foo");
            let other_str = other.to_string_lossy().to_string();
            let mut sp = SessionPermissions::default();
            sp.set_path_mode(&cwd.to_string_lossy(), PermissionMode::Read);

            // Use a non-temp path: temp dirs are always-allowed scratch, so
            // they wouldn't exercise the arg-path gate this test covers.
            let result = check_permission(
                "Bash", Some(&format!("ls {other_str}")), None,
                &cwd, &sp, None,
            );
            assert!(
                matches!(result, PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { .. })),
                "ls {other_str} from cwd {} should prompt — arg path not covered. got {result:?}",
                cwd.display(),
            );

            sp.set_path_mode(&home.join("b").to_string_lossy(), PermissionMode::Read);
            let result = check_permission(
                "Bash", Some(&format!("ls {other_str}")), None,
                &cwd, &sp, None,
            );
            assert!(matches!(result, PermissionCheckResult::Allowed),
                "ls {other_str} with read on ~/b should be allowed. got {result:?}");
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
                "Bash", Some("cat ../other-xyz/id_rsa"), None, &home, &sp, None,
            );
            assert!(
                matches!(escape, PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling { .. })),
                "cat ../other-xyz/id_rsa must prompt — it climbs out of the granted cwd. got {escape:?}",
            );

            if let Some(parent) = home.parent() {
                sp.set_path_mode(&parent.to_string_lossy(), PermissionMode::Read);
                let granted = check_permission(
                    "Bash", Some("cat ../other-xyz/id_rsa"), None, &home, &sp, None,
                );
                assert!(matches!(granted, PermissionCheckResult::Allowed),
                    "with read on the parent, the escaped read should pass. got {granted:?}");
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
        assert!(paths.is_empty(), "jq // should not be extracted; got {paths:?}");

        // Real paths still come through; bare-slash junk is dropped.
        let mixed = extract_command_paths("cat ~/.linggen/x.json / // ///");
        assert_eq!(mixed, vec!["~/.linggen/x.json".to_string()]);

        // Path-like words INSIDE a quoted string arg (a search query) are
        // NOT path args — the quoted span stays one word.
        let q = extract_command_paths(
            "ling-mem search \"binary path /usr/local/bin ~/.local/bin state ~/.linggen\" --limit 8",
        );
        assert!(q.is_empty(), "quoted search query must not yield path args; got {q:?}");

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
            matches!(check_permission("Bash", Some(scan), None, &cwd, &sp, None),
                     PermissionCheckResult::Allowed),
            "scan.sh under admin ~/.linggen should be allowed",
        );

        let ttl = "set -e; TTL_DAYS=$(curl -s http://127.0.0.1:9528/api/config 2>/dev/null \
                   | jq -r '.data.episodic_ttl_days // env.LING_MEM_EPISODIC_TTL_DAYS // 7'); \
                   ling-mem list --episodic --older-than \"${TTL_DAYS}d\" --format json \
                   | jq -c 'del(.vector)'";
        assert!(
            matches!(check_permission("Bash", Some(ttl), None, &cwd, &sp, None),
                     PermissionCheckResult::Allowed),
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
        sp2.set_path_mode(&dirs::home_dir().unwrap().join(".linggen").to_string_lossy(), PermissionMode::Admin);
        assert!(
            matches!(check_permission("Bash", Some(cmd), None, &cwd, &sp2, None),
                     PermissionCheckResult::Allowed),
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
        // Both pinned at Chat — see capabilities.rs::memory_capability for why.
        assert_eq!(tool_action_tier("Memory_query"), PermissionMode::Chat);
        assert_eq!(tool_action_tier("Memory_write"), PermissionMode::Chat);
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

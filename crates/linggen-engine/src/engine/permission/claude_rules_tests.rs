//! The deny and ask rules of the person's Claude Code settings, through the
//! one permission check (permission-spec.md § Claude Code's rules).

use super::claude_rules::ClaudeRules;
use super::model::check_permission_under_rules;
use super::{
    PathMode, PermissionCheckResult, PermissionMode, PromptKind, ReadOrigin, SessionPermissions,
};
use std::fs;
use std::path::{Path, PathBuf};

/// A project and a home of its own, outside the OS temp folder (a scratch
/// path there is free and would hide what a rule does).
struct World {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
}

impl World {
    fn new() -> Self {
        let real = dirs::home_dir().unwrap();
        Self {
            home: tempfile::tempdir_in(&real).unwrap(),
            project: tempfile::tempdir_in(&real).unwrap(),
        }
    }

    fn project(&self) -> &Path {
        self.project.path()
    }

    fn write(&self, file: &Path, json: serde_json::Value) {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, json.to_string()).unwrap();
    }

    fn user_file(&self) -> PathBuf {
        self.home.path().join(".claude/settings.json")
    }

    fn project_file(&self) -> PathBuf {
        self.project().join(".claude/settings.json")
    }

    fn local_file(&self) -> PathBuf {
        self.project().join(".claude/settings.local.json")
    }

    fn rules(&self) -> ClaudeRules {
        ClaudeRules::load(Some(self.home.path()), self.project())
    }

    fn at(&self, rel: &str) -> String {
        self.project().join(rel).to_string_lossy().into_owned()
    }
}

fn grants(path: &str, mode: PermissionMode) -> SessionPermissions {
    SessionPermissions {
        path_modes: vec![PathMode {
            path: path.into(),
            mode,
        }],
        ..Default::default()
    }
}

struct Call<'a> {
    tool: &'a str,
    command: Option<&'a str>,
    path: Option<String>,
    url: Option<&'a str>,
}

fn bash(command: &str) -> Call<'_> {
    Call {
        tool: "Bash",
        command: Some(command),
        path: None,
        url: None,
    }
}

fn file(tool: &str, path: String) -> Call<'_> {
    Call {
        tool,
        command: None,
        path: Some(path),
        url: None,
    }
}

fn check(
    world: &World,
    rules: &ClaudeRules,
    perms: &SessionPermissions,
    call: &Call<'_>,
    coding: bool,
) -> PermissionCheckResult {
    check_with(world, rules, perms, call, coding, &[])
}

fn check_with(
    world: &World,
    rules: &ClaudeRules,
    perms: &SessionPermissions,
    call: &Call<'_>,
    coding: bool,
    approved: &[PathBuf],
) -> PermissionCheckResult {
    let path = call
        .path
        .clone()
        .unwrap_or_else(|| world.project().to_string_lossy().into_owned());
    check_permission_under_rules(
        call.tool,
        call.command,
        Some(&path),
        world.project(),
        perms,
        None,
        coding,
        None,
        ReadOrigin::Other,
        None,
        approved,
        rules.for_call(call.url),
    )
}

fn refusal(result: &PermissionCheckResult) -> Option<&str> {
    match result {
        PermissionCheckResult::Blocked(reason) => Some(reason),
        _ => None,
    }
}

/// The one-time ask a rule makes: its reason, and whether it offered a grant.
fn rule_ask(result: &PermissionCheckResult) -> Option<(&str, bool)> {
    match result {
        PermissionCheckResult::NeedsPrompt(PromptKind::ExceedsCeiling {
            by_rule: true,
            reason,
            directories,
            ..
        }) => Some((reason.as_deref().unwrap_or(""), !directories.is_empty())),
        _ => None,
    }
}

fn admin_everywhere() -> SessionPermissions {
    grants("*", PermissionMode::Admin)
}

#[test]
fn a_deny_rule_refuses_the_read_of_the_file_it_names() {
    let world = World::new();
    world.write(
        &world.project_file(),
        serde_json::json!({"permissions": {"deny": ["Read(./.env)"]}}),
    );
    let rules = world.rules();
    let perms = admin_everywhere();
    for target in [".env", "app/.env"] {
        let result = check(
            &world,
            &rules,
            &perms,
            &file("Read", world.at(target)),
            true,
        );
        let reason = refusal(&result).unwrap_or_else(|| panic!("{target}: {result:?}"));
        assert!(
            reason.contains("`Read(./.env)`") && reason.contains(".claude/settings.json"),
            "{reason}"
        );
    }
    let other = check(
        &world,
        &rules,
        &perms,
        &file("Read", world.at("README.md")),
        true,
    );
    assert!(matches!(other, PermissionCheckResult::Allowed), "{other:?}");
}

#[test]
fn a_denied_glob_refuses_every_path_it_matches() {
    let world = World::new();
    world.write(
        &world.project_file(),
        serde_json::json!({"permissions": {"deny": ["Read(./secrets/**)", "Edit(/src/**/*.rs)", "Read(~/.aws/**)"]}}),
    );
    let rules = world.rules();
    let perms = admin_everywhere();
    let refused = |call: Call<'_>| refusal(&check(&world, &rules, &perms, &call, true)).is_some();
    assert!(refused(file("Read", world.at("secrets/a/key.pem"))));
    assert!(
        refused(file("Read", world.at("deep/secrets/key.pem"))),
        "one folder's `name/**` matches at any depth"
    );
    assert!(refused(file("Glob", world.at("secrets"))));
    assert!(refused(file("Grep", world.at("secrets/a"))));
    assert!(refused(file("Write", world.at("src/engine/x.rs"))));
    assert!(refused(file("Edit", world.at("src/x.rs"))));
    assert!(refused(file(
        "Read",
        world
            .home
            .path()
            .join(".aws/credentials")
            .to_string_lossy()
            .into_owned()
    )));
    assert!(
        refused(bash("cat secrets/a/key.pem")),
        "a read rule judges the files a command names"
    );
    assert!(
        refused(bash("cp notes.txt src/x.rs")),
        "an edit rule judges the files a writing command names"
    );
    assert!(
        !refused(bash("cat src/x.rs")),
        "an edit rule leaves a read of the file alone"
    );
    assert!(!refused(file("Write", world.at("docs/x.rs"))));
    assert!(!refused(file("Read", world.at("src/x.rs"))));
}

#[test]
fn a_denied_bash_prefix_refuses_the_command_however_it_is_run() {
    let world = World::new();
    world.write(
        &world.project_file(),
        serde_json::json!({"permissions": {"deny": ["Bash(rm -rf:*)", "Bash(git push * main)"]}}),
    );
    let rules = world.rules();
    let perms = admin_everywhere();
    for cmd in [
        "rm -rf build",
        "cd app && rm -rf build",
        "ls; rm -rf build",
        "FOO=1 rm -rf build",
        "env rm -rf build",
        "/bin/rm -rf build",
        "timeout 5 rm -rf build",
        "nice -n 5 rm -rf build",
        "sh -c 'rm -rf build'",
        "git push origin main",
    ] {
        let result = check(&world, &rules, &perms, &bash(cmd), true);
        let reason = refusal(&result).unwrap_or_else(|| panic!("{cmd}: {result:?}"));
        assert!(
            reason.starts_with("Refused by Claude Code settings:"),
            "{reason}"
        );
    }
    for cmd in ["rm build", "echo rm -rf build", "git push origin feature"] {
        let result = check(&world, &rules, &perms, &bash(cmd), true);
        assert!(refusal(&result).is_none(), "{cmd}: {result:?}");
    }
}

#[test]
fn a_deny_from_each_settings_file_is_honoured_and_names_its_file() {
    let world = World::new();
    world.write(
        &world.user_file(),
        serde_json::json!({"permissions": {"deny": ["Bash(git push:*)"]}}),
    );
    world.write(
        &world.project_file(),
        serde_json::json!({"permissions": {"deny": ["WebFetch"]}}),
    );
    world.write(
        &world.local_file(),
        serde_json::json!({"permissions": {"deny": ["Write(./build.rs)"]}}),
    );
    let rules = world.rules();
    let perms = admin_everywhere();
    let fetch = Call {
        tool: "WebFetch",
        command: None,
        path: None,
        url: Some("https://example.com"),
    };
    for (call, from) in [
        (bash("git push"), "/.claude/settings.json"),
        (fetch, "/.claude/settings.json"),
        (
            file("Write", world.at("build.rs")),
            "/.claude/settings.local.json",
        ),
    ] {
        let result = check(&world, &rules, &perms, &call, true);
        let reason = refusal(&result).unwrap_or_else(|| panic!("{}: {result:?}", call.tool));
        assert!(reason.contains(from), "{reason}");
    }
    let user = check(&world, &rules, &perms, &bash("git push"), true);
    let shown = super::display_path(&world.user_file());
    assert!(refusal(&user).unwrap().contains(&shown), "{user:?}");
}

#[test]
fn a_deny_rule_is_not_overridden_by_an_admin_grant() {
    let world = World::new();
    world.write(
        &world.user_file(),
        serde_json::json!({"permissions": {"deny": ["Read(//etc/hosts)", "Bash(make deploy)"]}}),
    );
    let rules = world.rules();
    for perms in [
        admin_everywhere(),
        grants(&world.project().to_string_lossy(), PermissionMode::Admin),
        grants("/etc", PermissionMode::Admin),
    ] {
        let read = check(
            &world,
            &rules,
            &perms,
            &file("Read", "/etc/hosts".into()),
            true,
        );
        assert!(refusal(&read).is_some(), "{read:?}");
        let run = check(&world, &rules, &perms, &bash("make deploy"), true);
        assert!(
            refusal(&run).is_some(),
            "a refusal, never a question a click could answer: {run:?}"
        );
    }
}

#[test]
fn an_ask_rule_asks_once_where_the_grants_would_allow() {
    let world = World::new();
    world.write(
        &world.project_file(),
        serde_json::json!({"permissions": {"ask": ["Bash(git push:*)", "Read(./notes/**)"]}}),
    );
    let rules = world.rules();
    let perms = admin_everywhere();
    let push = check(
        &world,
        &rules,
        &perms,
        &bash("git push origin feature"),
        true,
    );
    let (reason, grant_offered) = rule_ask(&push).unwrap_or_else(|| panic!("{push:?}"));
    assert!(
        reason.contains("`Bash(git push:*)`") && !grant_offered,
        "{reason}"
    );
    let note = world.at("notes/a.md");
    let read = check(&world, &rules, &perms, &file("Read", note.clone()), true);
    assert!(rule_ask(&read).is_some_and(|(_, grant)| !grant), "{read:?}");
    // Allowed once, the read goes ahead — and so does what a search finds inside it.
    let once = [super::sensitive::resolved(Path::new(&world.at("notes")))];
    let read = check_with(&world, &rules, &perms, &file("Read", note), true, &once);
    assert!(matches!(read, PermissionCheckResult::Allowed), "{read:?}");
    let other = check(&world, &rules, &perms, &bash("git status"), true);
    assert!(matches!(other, PermissionCheckResult::Allowed), "{other:?}");
}

#[test]
fn an_allow_rule_in_a_settings_file_grants_nothing() {
    let world = World::new();
    let everything = serde_json::json!({"permissions": {
        "allow": ["Bash", "Bash(*)", "Read", "Edit", "Write", "WebFetch", "*", "Read(//**)", "Edit(//**)"],
        "defaultMode": "bypassPermissions",
    }});
    world.write(&world.project_file(), everything.clone());
    world.write(&world.local_file(), everything.clone());
    world.write(&world.user_file(), everything);
    let rules = world.rules();
    let none = ClaudeRules::default();
    let perms = grants(&world.project().to_string_lossy(), PermissionMode::Read);
    for call in [
        file("Write", world.at("x.rs")),
        file("Read", "/etc/hosts".into()),
        bash("touch x"),
        bash("curl https://example.com"),
        bash("cargo publish"),
    ] {
        let with_files = check(&world, &rules, &perms, &call, true);
        assert!(
            !matches!(with_files, PermissionCheckResult::Allowed),
            "{}: {with_files:?}",
            call.tool
        );
        let without = check(&world, &none, &perms, &call, true);
        assert_eq!(format!("{with_files:?}"), format!("{without:?}"));
    }
}

#[test]
fn a_rule_not_understood_is_shown_and_a_deny_errs_strict() {
    let world = World::new();
    world.write(
        &world.project_file(),
        serde_json::json!({"permissions": {
            "deny": ["WebFetch(github.com)", "Frobnicate", "Read(!sample.env)"],
            "ask": ["Bash(command:ls *)"],
        }}),
    );
    world.write(
        &world.local_file(),
        serde_json::json!("not a settings object"),
    );
    fs::write(
        world
            .user_file()
            .parent()
            .map(|d| {
                fs::create_dir_all(d).unwrap();
                d.join("settings.json")
            })
            .unwrap(),
        "{ broken",
    )
    .unwrap();
    let rules = world.rules();
    let notice = rules.notice().expect("a line is shown");
    assert!(
        notice.contains("(unreadable)") && notice.contains("2 more"),
        "{notice}"
    );
    let listed = rules.not_understood().join(" ");
    for shown in [
        "`WebFetch(github.com)`",
        "`Frobnicate`",
        "`Read(!sample.env)`",
        "`Bash(command:ls *)`",
        "(unreadable)",
    ] {
        assert!(listed.contains(shown), "{shown}: {listed}");
    }
    let perms = admin_everywhere();
    let fetch = Call {
        tool: "WebFetch",
        command: None,
        path: None,
        url: Some("https://example.org/x"),
    };
    assert!(
        refusal(&check(&world, &rules, &perms, &fetch, true)).is_some(),
        "a deny not understood refuses its whole tool"
    );
    let ls = check(&world, &rules, &perms, &bash("ls -la"), true);
    assert!(
        matches!(ls, PermissionCheckResult::Allowed),
        "an ask not understood is not used: {ls:?}"
    );
}

#[test]
fn a_fetch_and_an_mcp_tool_are_matched_by_host_and_server() {
    let world = World::new();
    world.write(&world.user_file(), serde_json::json!({"permissions": {"deny": ["WebFetch(domain:*.example.com)", "mcp__files"]}}));
    let rules = world.rules();
    let perms = admin_everywhere();
    let fetch = |url| Call {
        tool: "WebFetch",
        command: None,
        path: None,
        url: Some(url),
    };
    assert!(refusal(&check(
        &world,
        &rules,
        &perms,
        &fetch("https://api.Example.com./x"),
        true
    ))
    .is_some());
    assert!(refusal(&check(
        &world,
        &rules,
        &perms,
        &fetch("https://example.org/"),
        true
    ))
    .is_none());
    let mcp = |tool| Call {
        tool,
        command: None,
        path: None,
        url: None,
    };
    assert!(refusal(&check(
        &world,
        &rules,
        &perms,
        &mcp("mcp__files__read"),
        true
    ))
    .is_some());
    assert!(refusal(&check(
        &world,
        &rules,
        &perms,
        &mcp("mcp__filesystem__read"),
        true
    ))
    .is_none());
}

#[test]
fn without_settings_files_or_outside_a_coding_session_the_check_is_as_today() {
    let world = World::new();
    let none = ClaudeRules::default();
    let empty = world.rules();
    assert!(empty.notice().is_none());
    let calls = || {
        vec![
            file("Read", world.at("a.rs")),
            file("Write", world.at("a.rs")),
            file("Read", "/etc/hosts".into()),
            bash("ls"),
            bash("rm -rf build"),
            bash("git push origin main"),
            bash("sudo ls"),
        ]
    };
    let perms_list = [
        SessionPermissions::default(),
        grants(&world.project().to_string_lossy(), PermissionMode::Edit),
        admin_everywhere(),
    ];
    for perms in &perms_list {
        for coding in [true, false] {
            for call in calls() {
                let today = format!("{:?}", check(&world, &none, perms, &call, coding));
                assert_eq!(
                    format!("{:?}", check(&world, &empty, perms, &call, coding)),
                    today
                );
            }
        }
    }
    // A session that is not a coding session does not take the rules in.
    world.write(
        &world.project_file(),
        serde_json::json!({"permissions": {"deny": ["Bash", "Read", "Edit"], "ask": ["*"]}}),
    );
    let rules = world.rules();
    for perms in &perms_list {
        for call in calls() {
            let today = format!("{:?}", check(&world, &none, perms, &call, false));
            assert_eq!(
                format!("{:?}", check(&world, &rules, perms, &call, false)),
                today
            );
        }
    }
}

#[test]
fn a_rule_in_the_git_roots_settings_holds_in_a_folder_below_it() {
    let world = World::new();
    fs::create_dir_all(world.project().join(".git")).unwrap();
    let sub = world.project().join("crates/app");
    fs::create_dir_all(&sub).unwrap();
    world.write(
        &world.project_file(),
        serde_json::json!({"permissions": {"deny": ["Read(./.env)", "Read(/secrets/**)"]}}),
    );
    let rules = ClaudeRules::load(Some(world.home.path()), &sub);
    let perms = admin_everywhere();
    let read = |path: PathBuf| {
        check_permission_under_rules(
            "Read",
            None,
            path.to_str(),
            &sub,
            &perms,
            None,
            true,
            None,
            ReadOrigin::Other,
            None,
            &[],
            rules.for_call(None),
        )
    };
    for target in [
        world.project().join(".env"),
        world.project().join("secrets/key"),
        sub.join(".env"),
    ] {
        let result = read(target.clone());
        assert!(
            refusal(&result).is_some(),
            "{}: {result:?}",
            target.display()
        );
    }
    assert!(matches!(
        read(sub.join("main.rs")),
        PermissionCheckResult::Allowed
    ));
}

#[test]
fn a_denied_file_is_refused_by_any_name_that_reaches_it() {
    let world = World::new();
    fs::write(world.project().join(".env"), "KEY=1\n").unwrap();
    std::os::unix::fs::symlink(
        world.project().join(".env"),
        world.project().join("settings.txt"),
    )
    .unwrap();
    world.write(
        &world.project_file(),
        serde_json::json!({"permissions": {"deny": ["Read(./.env)"]}}),
    );
    let rules = world.rules();
    let perms = admin_everywhere();
    for name in [".ENV", "settings.txt", "./sub/../.env"] {
        let result = check(&world, &rules, &perms, &file("Read", world.at(name)), true);
        assert!(refusal(&result).is_some(), "{name}: {result:?}");
    }
}

#[test]
fn the_line_for_rules_not_understood_names_three_and_counts_the_rest() {
    let world = World::new();
    let junk: Vec<String> = (0..5)
        .map(|i| format!("Unknown{i}{}", "x".repeat(200)))
        .collect();
    world.write(
        &world.project_file(),
        serde_json::json!({"permissions": {"deny": junk}}),
    );
    let notice = world.rules().notice().unwrap();
    assert!(notice.contains("2 more"), "{notice}");
    assert!(!notice.contains("Unknown3"), "{notice}");
    assert!(notice.len() < 500, "{notice}");
}

// ---------------------------------------------------------------------------
// A change to a file the rules are read from is asked every time
// (permission-spec.md § Claude Code's rules).
// ---------------------------------------------------------------------------

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// The calls that change `file`: both file tools, and commands that write.
fn changes_of(target: &Path) -> Vec<Call<'static>> {
    let file_text = path_text(target);
    let command = |cmd: String| -> Call<'static> {
        Call {
            tool: "Bash",
            command: Some(Box::leak(cmd.into_boxed_str())),
            path: None,
            url: None,
        }
    };
    vec![
        file("Write", file_text.clone()),
        file("Edit", file_text.clone()),
        command(format!("rm {file_text}")),
        command(format!("mv {file_text} {file_text}.old")),
        command(format!("echo '{{}}' > {file_text}")),
        command(format!("sed -i 's/a/b/' {file_text}")),
    ]
}

fn asked_once(result: &PermissionCheckResult) -> bool {
    rule_ask(result).is_some_and(|(reason, grant)| !grant && reason.contains("settings file"))
}

#[test]
fn a_change_to_a_settings_file_asks_once_at_admin_everywhere() {
    let world = World::new();
    // No rule in any file, and the files need not exist yet.
    let rules = world.rules();
    let perms = admin_everywhere();
    for file in [world.user_file(), world.project_file(), world.local_file()] {
        for call in changes_of(&file) {
            let result = check(&world, &rules, &perms, &call, true);
            assert!(
                asked_once(&result),
                "{} {:?} {}: {result:?}",
                call.tool,
                call.command,
                file.display()
            );
        }
    }
}

#[test]
fn a_change_to_a_settings_file_asks_whatever_the_session_holds_and_wherever_it_runs() {
    let world = World::new();
    world.write(
        &world.project_file(),
        serde_json::json!({"permissions": {"deny": ["Read(./.env)"]}}),
    );
    let rules = world.rules();
    for coding in [true, false] {
        for perms in [
            admin_everywhere(),
            grants(&path_text(world.project()), PermissionMode::Admin),
            grants(&path_text(world.project()), PermissionMode::Edit),
        ] {
            for call in changes_of(&world.project_file()) {
                let result = check(&world, &rules, &perms, &call, coding);
                assert!(
                    asked_once(&result),
                    "coding {coding} {call:?}: {result:?}",
                    call = call.command
                );
            }
        }
    }
}

#[test]
fn a_settings_file_the_run_starts_below_is_asked_at_the_git_root_too() {
    let world = World::new();
    fs::create_dir_all(world.project().join(".git")).unwrap();
    let sub = world.project().join("crates/app");
    fs::create_dir_all(&sub).unwrap();
    let rules = ClaudeRules::load(Some(world.home.path()), &sub);
    let perms = admin_everywhere();
    let change = |path: PathBuf| {
        check_permission_under_rules(
            "Write",
            None,
            path.to_str(),
            &sub,
            &perms,
            None,
            true,
            None,
            ReadOrigin::Other,
            None,
            &[],
            rules.for_call(None),
        )
    };
    for file in [
        world.project().join(".claude/settings.json"),
        world.project().join(".claude/settings.local.json"),
        sub.join(".claude/settings.json"),
        sub.join(".claude/settings.local.json"),
        world.home.path().join(".claude/settings.json"),
    ] {
        let result = change(file.clone());
        assert!(asked_once(&result), "{}: {result:?}", file.display());
    }
}

#[test]
fn a_settings_file_is_asked_by_any_name_that_reaches_it() {
    let world = World::new();
    world.write(&world.project_file(), serde_json::json!({}));
    std::os::unix::fs::symlink(world.project_file(), world.project().join("notes.txt")).unwrap();
    let rules = world.rules();
    let perms = admin_everywhere();
    for name in [
        ".CLAUDE/Settings.JSON",
        "notes.txt",
        "sub/../.claude/settings.json",
        "./.claude/settings.json",
    ] {
        let result = check(&world, &rules, &perms, &file("Write", world.at(name)), true);
        assert!(asked_once(&result), "{name}: {result:?}");
    }
    // Removing or moving the folder takes the file with it.
    for cmd in [
        "rm -rf .claude",
        "mv .claude .claude.old",
        "rm -rf ./.claude/",
    ] {
        let result = check(&world, &rules, &perms, &bash(cmd), true);
        assert!(asked_once(&result), "{cmd}: {result:?}");
    }
}

#[test]
fn reading_a_settings_file_and_changing_its_neighbours_is_not_asked() {
    let world = World::new();
    world.write(&world.project_file(), serde_json::json!({}));
    let rules = world.rules();
    let perms = admin_everywhere();
    let project = path_text(&world.project_file());
    for tool in ["Read", "Glob", "Grep"] {
        let result = check(&world, &rules, &perms, &file(tool, project.clone()), true);
        assert!(
            matches!(result, PermissionCheckResult::Allowed),
            "{tool}: {result:?}"
        );
    }
    for cmd in [
        format!("cat {project}"),
        format!("grep permissions {project}"),
        format!("jq . {project}"),
        format!("ls -la {}", world.at(".claude")),
    ] {
        let result = check(&world, &rules, &perms, &bash(&cmd), true);
        assert!(
            matches!(result, PermissionCheckResult::Allowed),
            "{cmd}: {result:?}"
        );
    }
    for target in [
        ".claude/commands/review.md",
        ".claude/settings.json.bak",
        "sub/.claude/settings.json",
        "settings.json",
        "src/main.rs",
    ] {
        let result = check(
            &world,
            &rules,
            &perms,
            &file("Write", world.at(target)),
            true,
        );
        assert!(
            matches!(result, PermissionCheckResult::Allowed),
            "{target}: {result:?}"
        );
    }
}

#[test]
fn a_deny_rule_still_refuses_a_change_to_a_settings_file() {
    let world = World::new();
    world.write(
        &world.project_file(),
        serde_json::json!({"permissions": {"deny": ["Edit(./.claude/settings.local.json)"]}}),
    );
    let rules = world.rules();
    let perms = admin_everywhere();
    let result = check(
        &world,
        &rules,
        &perms,
        &file("Write", path_text(&world.local_file())),
        true,
    );
    assert!(
        refusal(&result).is_some(),
        "a refusal stays a refusal: {result:?}"
    );
}

/// The files the rules are read from and the files that are asked are one
/// list: a rule written in any file `load` reads is honoured, and the same
/// file is asked about. A second list of names anywhere in the engine fails
/// the scan below.
#[test]
fn the_files_asked_about_are_the_files_the_rules_are_read_from() {
    let world = World::new();
    fs::create_dir_all(world.project().join(".git")).unwrap();
    let sub = world.project().join("crates/app");
    fs::create_dir_all(&sub).unwrap();
    let files = [
        world.home.path().join(".claude/settings.json"),
        sub.join(".claude/settings.json"),
        sub.join(".claude/settings.local.json"),
        world.project().join(".claude/settings.json"),
        world.project().join(".claude/settings.local.json"),
    ];
    for (i, file) in files.iter().enumerate() {
        world.write(
            file,
            serde_json::json!({"permissions": {"deny": [format!("Bash(marker{i})")]}}),
        );
    }
    let rules = ClaudeRules::load(Some(world.home.path()), &sub);
    let perms = admin_everywhere();
    let run = |tool: &str, command: Option<&str>, path: &Path| {
        check_permission_under_rules(
            tool,
            command,
            path.to_str(),
            &sub,
            &perms,
            None,
            true,
            None,
            ReadOrigin::Other,
            None,
            &[],
            rules.for_call(None),
        )
    };
    for (i, file) in files.iter().enumerate() {
        let read_from = run("Bash", Some(&format!("marker{i}")), &sub);
        assert!(
            refusal(&read_from).is_some(),
            "{} is not read: {read_from:?}",
            file.display()
        );
        let asked = run("Write", None, file);
        assert!(
            asked_once(&asked),
            "{} is not asked about: {asked:?}",
            file.display()
        );
    }
}

fn engine_sources(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            engine_sources(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
}

/// The engine's source files, as (path, text), that name the settings
/// files. Test files list the names in their own checks and are left out.
fn naming_the_settings_files(sources: &[(PathBuf, String)]) -> Vec<&Path> {
    sources
        .iter()
        .filter(|(path, text)| {
            !path.to_string_lossy().ends_with("_tests.rs")
                && text.contains("\"settings.local.json\"")
        })
        .map(|(path, _)| path.as_path())
        .collect()
}

#[test]
fn the_settings_files_are_named_in_one_place() {
    let engine = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut paths = Vec::new();
    engine_sources(&engine.join("src"), &mut paths);
    engine_sources(&engine.join("../../src"), &mut paths);
    let sources: Vec<_> = paths
        .into_iter()
        .map(|path| {
            let text = fs::read_to_string(&path).unwrap();
            (path, text)
        })
        .collect();
    let naming = naming_the_settings_files(&sources);
    assert_eq!(naming.len(), 1, "one place names the files: {naming:?}");
    assert!(
        naming[0].ends_with("permission/claude_rules.rs"),
        "{naming:?}"
    );
}

#[test]
fn the_settings_files_lock_sees_a_second_list() {
    let copy = (
        PathBuf::from("src/engine/other.rs"),
        "let f = \"settings.local.json\";".to_string(),
    );
    let own = (
        PathBuf::from("src/engine/permission/claude_rules.rs"),
        "[\"settings.json\", \"settings.local.json\"]".to_string(),
    );
    assert_eq!(naming_the_settings_files(&[own, copy]).len(), 2);
}

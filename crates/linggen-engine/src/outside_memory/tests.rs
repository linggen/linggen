//! The reader registry, the slice, the confinement and the redaction, on
//! made-up folders in a scratch home. No real note text, no real path.

use super::sources::encode_project_path;
use super::*;
use std::fs;

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// A scratch home, and its `Env`.
fn scratch() -> (tempfile::TempDir, Env) {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap();
    let env = Env::new(home);
    (dir, env)
}

/// Claude Code's memory folder for a project at `project` (under the home).
fn cc_memory(env: &Env, project: &Path) -> PathBuf {
    env.home()
        .join(".claude/projects")
        .join(encode_project_path(project))
        .join("memory")
}

fn index_of(titles: &[(&str, &str)]) -> String {
    let mut text = String::from("# Notes\n\n## Rules\n");
    for (title, file) in titles {
        text.push_str(&format!("- [{title}]({file}) — about {title}\n"));
    }
    text
}

// ── paths ──────────────────────────────────────────────────────────────────

#[test]
fn a_project_folder_is_found_by_encoding_forward() {
    assert_eq!(encode_project_path(Path::new("/Users/a/work/app")), "-Users-a-work-app");
    // A dot and a slash side by side are two dashes.
    assert_eq!(encode_project_path(Path::new("/Users/a/.cache/x_y")), "-Users-a--cache-x-y");
    // A character outside ASCII is one dash (two when it is outside the BMP).
    assert_eq!(encode_project_path(Path::new("/a/项目")), "-a---");
    assert_eq!(encode_project_path(Path::new("/a/😀")), "-a---");
}

#[test]
fn the_chain_runs_nearest_first_and_stops_before_home_and_root() {
    let (_d, env) = scratch();
    let cwd = env.home().join("work/app/sub");
    let chain = project_chain_of(&env, &cwd);
    assert_eq!(
        chain,
        [cwd.clone(), env.home().join("work/app"), env.home().join("work")]
    );
    let outside = project_chain_of(&env, Path::new("/zz_not_home/a"));
    assert_eq!(outside, [PathBuf::from("/zz_not_home/a"), PathBuf::from("/zz_not_home")]);
}

fn project_chain_of(env: &Env, cwd: &Path) -> Vec<PathBuf> {
    super::sources::project_chain(env, cwd)
}

// ── the slice ──────────────────────────────────────────────────────────────

#[test]
fn codex_is_forwarded_what_claude_code_remembers_for_the_nearest_projects_first() {
    let (_d, env) = scratch();
    let near = env.home().join("work/app");
    let far = env.home().join("work");
    put(&cc_memory(&env, &near).join("MEMORY.md"), &index_of(&[("Near rule", "near.md")]));
    put(&cc_memory(&env, &far).join("MEMORY.md"), &index_of(&[("Far rule", "far.md")]));
    let cwd = near.join("src");
    let got = slice(&env, Some(Agent::Codex), Some(&cwd));
    let block = got.block().into_string();
    let near_at = block.find("Near rule").expect("near line");
    let far_at = block.find("Far rule").expect("far line");
    assert!(near_at < far_at, "{block}");
    assert!(block.contains("Claude Code · project ~/work/app"), "{block}");
    assert!(block.contains("notes in ~/.claude/projects/"), "{block}");
    assert_eq!(got.cut, 0);
}

#[test]
fn an_agent_is_not_forwarded_its_own_memory_and_an_unknown_host_gets_everyones() {
    let (_d, env) = scratch();
    let project = env.home().join("work/app");
    put(&cc_memory(&env, &project).join("MEMORY.md"), &index_of(&[("A rule", "a.md")]));
    assert!(slice(&env, Some(Agent::ClaudeCode), Some(&project)).is_empty());
    assert!(!slice(&env, None, Some(&project)).is_empty());
    assert!(!slice(&env, Agent::of_host("cursor"), Some(&project)).is_empty());
    assert_eq!(Agent::of_host("claude-code"), Some(Agent::ClaudeCode));
    assert_eq!(Agent::of_host("Codex"), Some(Agent::Codex));
}

#[test]
fn the_person_is_forwarded_as_the_headings_of_claudes_own_file() {
    let (_d, env) = scratch();
    put(&env.home().join(".claude/CLAUDE.md"), "# Title\n\nbody text\n\n## First area\nmore\n## Second area\n");
    let got = slice(&env, Some(Agent::Codex), None);
    let block = got.block().into_string();
    assert!(block.contains("the person, from ~/.claude/CLAUDE.md"), "{block}");
    assert!(block.contains("- First area") && block.contains("- Second area"), "{block}");
    assert!(!block.contains("body text"), "{block}");
}

#[test]
fn the_budget_holds_keeps_the_persons_share_and_counts_what_it_left_out() {
    let (_d, env) = scratch();
    let project = env.home().join("work/app");
    let mut text = String::new();
    for n in 0..150 {
        text.push_str(&format!("- [Project note {n} 项目说明 {}](n{n}.md) — 这一条是中文说明，用来检查不会在字符中间截断\n", "x".repeat(40)));
    }
    put(&cc_memory(&env, &project).join("MEMORY.md"), &text);
    put(&env.home().join(".claude/CLAUDE.md"), "## Who I am\n## How I work\n");
    let got = slice(&env, Some(Agent::Codex), Some(&project));
    let block = got.block().into_string();
    assert!(block.len() <= BUDGET_BYTES, "{} bytes", block.len());
    assert!(got.cut > 0 && got.cut < 150 + 2, "cut {}", got.cut);
    assert!(block.contains("Who I am") && block.contains("How I work"), "the person kept a share: {block}");
    assert!(block.contains(&format!("{} more lines left out", got.cut)), "{block}");
}

#[test]
fn an_index_is_read_the_way_claude_code_loads_it() {
    let (_d, env) = scratch();
    let project = env.home().join("work/app");
    let mut text = String::new();
    for n in 0..260 {
        text.push_str(&format!("- [Note {n}](n{n}.md)\n"));
    }
    put(&cc_memory(&env, &project).join("MEMORY.md"), &text);
    let got = slice(&env, Some(Agent::Codex), Some(&project));
    // Only the first 200 lines are the index; the rest are not forwarded.
    assert_eq!(got.entries().len() + got.cut, 200);
}

#[test]
fn a_source_that_is_missing_or_unreadable_is_skipped_and_reported() {
    let (_d, env) = scratch();
    // Nothing at all: no error, nothing forwarded.
    assert!(slice(&env, None, Some(&env.home().join("work"))).is_empty());
    // Codex's folder has files, in a format not known yet: reported, never fatal.
    put(&env.home().join(".codex/memories/some-file.md"), "something");
    let states: Vec<_> = reports(&env, &Scope { cwd: None })
        .into_iter()
        .map(|r| (r.id, r.state))
        .collect();
    assert!(states.contains(&("codex-memories", "format not known yet".to_string())), "{states:?}");
    assert!(states.contains(&("claude-code-instructions", "no data".to_string())), "{states:?}");
    let got = slice(&env, Some(Agent::ClaudeCode), None);
    assert!(got.is_empty());
    assert!(got.notes.iter().any(|n| n.source == "codex-memories"), "{:?}", got.notes);
}

// ── read confinement ───────────────────────────────────────────────────────

fn with_a_note() -> (tempfile::TempDir, Env, PathBuf) {
    let (d, env) = scratch();
    let project = env.home().join("work/app");
    let folder = cc_memory(&env, &project);
    put(&folder.join("MEMORY.md"), &index_of(&[("Rule", "rule.md")]));
    put(&folder.join("rule.md"), "The rule, in full.\n");
    (d, env, folder)
}

#[test]
fn a_note_inside_a_source_is_read_by_either_spelling_of_its_path() {
    let (_d, env, folder) = with_a_note();
    let note = read(&env, folder.join("rule.md").to_str().unwrap()).unwrap();
    assert_eq!(note.source, "claude-code-project");
    assert!(note.text.as_str().contains("The rule, in full."));
    let tilde = env.tilde(&folder.join("rule.md"));
    assert!(tilde.starts_with("~/"), "{tilde}");
    assert!(read(&env, &tilde).is_ok());
}

#[test]
fn nothing_outside_a_registered_source_can_be_read() {
    let (_d, env, folder) = with_a_note();
    let project_dir = folder.parent().unwrap();
    put(&project_dir.join("session.jsonl"), "{\"prompt\":\"a transcript\"}\n");
    put(&project_dir.join("other.md"), "beside the memory folder, not in it");
    put(&env.home().join("secret.md"), "a file of the person's");
    let refused = ReadError::OutsideSources;
    for path in [
        project_dir.join("session.jsonl"),
        project_dir.join("other.md"),
        env.home().join("secret.md"),
        folder.join("../other.md"),
        folder.join("../../../secret.md"),
        folder.join("missing.md"),
        PathBuf::from("/etc/hosts"),
    ] {
        assert_eq!(read(&env, path.to_str().unwrap()).unwrap_err(), refused, "{path:?}");
    }
    assert_eq!(read(&env, "rule.md").unwrap_err(), ReadError::NotAbsolute);
    // A folder, and a file that is no note by its extension.
    put(&folder.join("data.json"), "{}");
    assert_eq!(read(&env, folder.to_str().unwrap()).unwrap_err(), refused);
    assert_eq!(read(&env, folder.join("data.json").to_str().unwrap()).unwrap_err(), refused);
}

#[test]
fn a_link_inside_a_source_cannot_lead_out_of_it() {
    let (_d, env, folder) = with_a_note();
    put(&env.home().join("elsewhere/private.md"), "outside every source");
    std::os::unix::fs::symlink(env.home().join("elsewhere/private.md"), folder.join("link.md")).unwrap();
    assert_eq!(
        read(&env, folder.join("link.md").to_str().unwrap()).unwrap_err(),
        ReadError::OutsideSources
    );
    assert!(search(&env, "outside", 5).is_empty(), "search does not follow links either");
}

#[test]
fn a_big_note_is_cut_and_says_so() {
    let (_d, env, folder) = with_a_note();
    put(&folder.join("big.md"), &"line of words\n".repeat(10_000));
    let note = read(&env, folder.join("big.md").to_str().unwrap()).unwrap();
    assert!(note.truncated);
    assert!(note.text.as_str().len() <= 64 * 1024);
}

// ── search ─────────────────────────────────────────────────────────────────

#[test]
fn search_finds_words_in_any_language_best_line_first() {
    let (_d, env, folder) = with_a_note();
    put(&folder.join("two.md"), "Gamma only\n伽马 delta 与 epsilon 同在\nepsilon alone\n");
    let hits = search(&env, "delta EPSILON", 10);
    assert_eq!(hits[0].text.as_str(), "伽马 delta 与 epsilon 同在");
    assert_eq!(hits[0].line, 2);
    assert_eq!(hits.len(), 2);
    assert_eq!(search(&env, "伽马", 10).len(), 1);
    assert!(search(&env, "   ", 10).is_empty());
    assert_eq!(search(&env, "epsilon", 1).len(), 1);
}

// ── redaction ──────────────────────────────────────────────────────────────

#[test]
fn key_shaped_strings_are_redacted_everywhere_they_could_leave() {
    let (_d, env, folder) = with_a_note();
    let key = format!("sk-{}", "a1B2".repeat(8));
    let hub = format!("ghp_{}", "Z9y8".repeat(6));
    put(
        &folder.join("rule.md"),
        &format!("use {key} and {hub}\napi_key = abcdef123456\n-----BEGIN RSA PRIVATE KEY-----\nMIIB\n-----END RSA PRIVATE KEY-----\nAuthorization: Bearer abcdefghijklmnop1234\n"),
    );
    put(
        &folder.join("MEMORY.md"),
        &format!("- [Rule](rule.md) — key {key}\n"),
    );
    let note = read(&env, folder.join("rule.md").to_str().unwrap()).unwrap();
    let hits = search(&env, "BEGIN", 5);
    let block = slice(&env, Some(Agent::Codex), Some(&env.home().join("work/app"))).block();
    for text in [note.text.as_str(), hits[0].text.as_str(), block.as_str()] {
        for secret in [key.as_str(), hub.as_str(), "abcdef123456", "MIIB", "abcdefghijklmnop1234"] {
            assert!(!text.contains(secret), "{secret} left in: {text}");
        }
    }
    assert!(note.text.as_str().contains("[redacted]"));
    assert!(block.as_str().contains("key [redacted]"), "{block}");
}

#[test]
fn file_names_and_ids_in_an_index_line_are_not_mistaken_for_keys() {
    let line = "- [Gotcha scratch home](gotcha_scratch_home_env_long_name.md) — id 09ba1d54-bbeb-41fc-9888-02973b8df176, row Mw13jg0tlu, task-list and risk-assessment-notes-for-the-team";
    assert_eq!(super::redact::clean(line).as_str(), line);
}

// ── locks ──────────────────────────────────────────────────────────────────

/// The module's own files without this one: what runs in production.
fn production_files() -> [(&'static str, &'static str); 5] {
    [
        ("mod.rs", include_str!("mod.rs")),
        ("access.rs", include_str!("access.rs")),
        ("redact.rs", include_str!("redact.rs")),
        ("slice.rs", include_str!("slice.rs")),
        ("sources.rs", include_str!("sources.rs")),
    ]
}

/// Nothing here can change another agent's files: no production file of the
/// module names a call that writes, moves, creates or removes.
#[test]
fn this_module_never_writes() {
    let writes = [
        "fs::write", "fs::remove", "fs::rename", "fs::copy", "fs::create", "fs::set_permissions",
        "File::create", "OpenOptions", "create_dir", "hard_link", "symlink(",
    ];
    for (name, text) in production_files() {
        // Doc and line comments may name them; code may not.
        let code: String = text
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for call in writes {
            assert!(!code.contains(call), "{name} names `{call}`");
        }
    }
}

/// Another agent's folder is named in the registry alone.
#[test]
fn only_the_source_list_names_other_agents_folders() {
    for (name, text) in production_files() {
        if name == "sources.rs" {
            continue;
        }
        let code: String = text
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for folder in [".claude", ".codex", ".hermes", ".gemini", ".openclaw"] {
            assert!(!code.contains(folder), "{name} names `{folder}`");
        }
    }
}

/// Text from another agent's files leaves this module as `Clean` only, and
/// only `redact::clean` makes one: the type's field is private to its file,
/// and no other production file builds the tuple struct.
#[test]
fn only_redact_builds_clean_text() {
    for (name, text) in production_files() {
        if name == "redact.rs" {
            continue;
        }
        assert!(!text.contains("Clean("), "{name} builds a Clean");
    }
}

#[test]
fn source_ids_are_distinct() {
    let mut ids: Vec<_> = super::sources::SOURCES.iter().map(|s| s.id).collect();
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n);
}

use super::foreground_shell::Ran;
use super::tool_helpers::{build_globset, to_rel_string};
use super::{SearchMatch, ToolResult, Tools};
use anyhow::Result;
use grep::regex::RegexMatcher;
use grep::searcher::sinks::UTF8;
use grep::searcher::Searcher;
use ignore::WalkBuilder;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

/// Walk up from `path` looking for a `.git` directory or file (worktrees),
/// never above `$HOME` (see [`project_ancestors`]). Home itself is never a
/// project root (a dotfiles repo there would claim everything below).
///
/// The one place the engine finds a project root: the permission card and a
/// declined scope ask it, and `the_engine_has_one_git_root_walk` fails on a
/// second walk. The memory crate keeps its own (`ling_mem::memory::scope::
/// find_root`, it cannot call up here); a test below holds the two equal.
pub fn find_git_root(path: &Path) -> Option<PathBuf> {
    let home = dirs::home_dir();
    git_root_within(path, home.as_deref())
}

fn git_root_within(path: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let found = project_ancestors_within(path, home)
        .into_iter()
        .find(|d| d.join(".git").exists())?;
    let is_home = home.is_some_and(|h| home_forms(h).iter().any(|f| f == found));
    (!is_home).then(|| found.to_path_buf())
}

/// The directories a project walk from `path` may read: `path` and its
/// ancestors, ending at `$HOME` when `path` is under it — nothing above the
/// person's home is ever read (a repo or a CLAUDE.md there belongs to
/// someone else's tree). A path outside home walks to the filesystem root.
pub fn project_ancestors(path: &Path) -> Vec<&Path> {
    let home = dirs::home_dir();
    project_ancestors_within(path, home.as_deref())
}

pub(crate) fn project_ancestors_within<'a>(path: &'a Path, home: Option<&Path>) -> Vec<&'a Path> {
    let all = path.ancestors();
    let Some(home) = home.and_then(|h| home_forms(h).into_iter().find(|f| path.starts_with(f)))
    else {
        return all.collect();
    };
    all.take_while(|d| d.starts_with(&home)).collect()
}

/// Home as given and canonicalized (`/var` → `/private/var` on macOS): a
/// canonical session root must still find the home it sits under.
fn home_forms(home: &Path) -> Vec<PathBuf> {
    let mut forms = vec![home.to_path_buf()];
    if let Ok(real) = home.canonicalize() {
        if real != home {
            forms.push(real);
        }
    }
    forms
}

#[derive(Debug, Deserialize)]
pub(super) struct SearchArgs {
    pub(super) query: String,
    pub(super) globs: Option<Vec<String>>,
    pub(super) max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RunCommandArgs {
    pub(super) cmd: String,
    pub(super) timeout_ms: Option<u64>,
    /// Start it beside the chat and return at once
    /// (doc/background-tasks-spec.md).
    #[serde(default)]
    pub(super) run_in_background: bool,
    #[serde(skip)]
    pub(super) cancel_flag: Option<Arc<AtomicBool>>,
}

/// SIGTERM then SIGKILL the child's whole process group. A shell command is
/// a pipeline, not one process — killing only the shell leaves its children
/// running and holding the output pipes open.
pub(crate) fn kill_process_group(child: &std::process::Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        unsafe {
            libc::killpg(pid, libc::SIGTERM);
        }
        std::thread::sleep(Duration::from_millis(100));
        unsafe {
            libc::killpg(pid, libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    {
        // On non-unix, we can't use killpg; the caller already uses child.kill().
        let _ = child;
    }
}

/// Raises its flag when it goes, at the end of a call or when the call is
/// dropped; a command still waited on is then killed.
struct CancelWhenDropped(Arc<AtomicBool>);

impl Drop for CancelWhenDropped {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Tools {
    pub(super) async fn search_rg(&self, args: SearchArgs) -> Result<ToolResult> {
        self.run_blocking("search_rg", move |tools| tools.search_rg_inner(args))
            .await
    }

    /// Watch a foreground Bash as a background task if Steer or the
    /// foreground limit moved it (`foreground_shell.rs`). Every command a
    /// tool runs in the foreground comes here — Bash, a `bash` app — so it
    /// runs off the runtime's workers, under the foreground limit, and is
    /// killed when its call is dropped (Stop drops it): the guard raises the
    /// flag the shell's wait reads.
    pub(super) async fn run_command(&self, mut args: RunCommandArgs) -> Result<ToolResult> {
        let cancel = args.cancel_flag.get_or_insert_with(Default::default).clone();
        let _cancel_when_dropped = CancelWhenDropped(cancel);
        let ran = self
            .run_blocking("run_command", move |tools| tools.run_foreground_shell(args))
            .await?;
        match ran {
            Ran::Done(result) => Ok(result),
            Ran::Moved(moved) => self.watch_moved(moved).await,
        }
    }

    fn search_rg_inner(&self, args: SearchArgs) -> Result<ToolResult> {
        let globset = build_globset(args.globs.as_deref())?;
        let max_results = args.max_results.unwrap_or(200);

        let matcher = RegexMatcher::new(&args.query).or_else(|_| {
            let escaped = regex::escape(&args.query);
            RegexMatcher::new(&escaped)
                .map_err(|e| anyhow::anyhow!("invalid search pattern '{}': {}", args.query, e))
        })?;

        let mut matches = Vec::new();
        let mut searcher = Searcher::new();
        let cwd = self.cwd();
        let mut walker = WalkBuilder::new(&cwd);
        walker.standard_filters(true).hidden(true);
        self.filter_read_walk(&mut walker);

        for entry in walker.build() {
            let entry = match entry {
                Ok(v) => v,
                Err(_) => continue,
            };
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }

            let path = entry.path();
            let rel_path = to_rel_string(&cwd, path)?;
            if let Some(gs) = &globset {
                if !gs.is_match(Path::new(&rel_path)) {
                    continue;
                }
            }
            let mut file_matches = Vec::new();

            let _ = searcher.search_path(
                &matcher,
                path,
                UTF8(|line_num, line_content| {
                    file_matches.push(SearchMatch {
                        path: rel_path.clone(),
                        line: line_num as usize,
                        snippet: line_content.trim_end().to_string(),
                    });
                    if matches.len() + file_matches.len() >= max_results {
                        return Ok(false);
                    }
                    Ok(true)
                }),
            );

            matches.extend(file_matches);
            if matches.len() >= max_results {
                break;
            }
        }

        Ok(ToolResult::SearchMatches(matches))
    }
}

#[cfg(test)]
mod project_walk_tests {
    use super::*;

    /// A repo above `$HOME` is someone else's tree: a walk from inside home
    /// stops at home and never finds it.
    #[test]
    fn the_walk_stops_at_home() {
        let dir = tempfile::tempdir().unwrap();
        let outer = dir.path().canonicalize().unwrap();
        std::fs::create_dir(outer.join(".git")).unwrap();
        let home = outer.join("home");
        let chat = home.join("work/chat");
        std::fs::create_dir_all(&chat).unwrap();
        assert_eq!(git_root_within(&chat, Some(&home)), None);
        let walked = project_ancestors_within(&chat, Some(&home));
        assert_eq!(walked.last().copied(), Some(home.as_path()));

        let repo = home.join("work/harbor");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        assert_eq!(git_root_within(&repo.join("src"), Some(&home)), Some(repo));

        std::fs::create_dir(home.join(".git")).unwrap();
        assert_eq!(
            git_root_within(&chat, Some(&home)),
            None,
            "home is never a root"
        );
        // Outside home the walk goes on up, as before.
        let elsewhere = outer.join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        assert_eq!(git_root_within(&elsewhere, Some(&home)), Some(outer));
    }

    /// `text` with each `#[cfg(test)] mod … { … }` cut out (braces counted).
    fn without_test_modules(text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(at) = rest.find("#[cfg(test)]") {
            out.push_str(&rest[..at]);
            let after = &rest[at + "#[cfg(test)]".len()..];
            if !after.trim_start().starts_with("mod ") {
                rest = after;
                continue;
            }
            let Some(open) = after.find('{') else {
                return out;
            };
            let mut depth = 0usize;
            let mut end = after.len();
            for (i, c) in after[open..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            rest = &after[end..];
        }
        out.push_str(rest);
        out
    }

    /// The lock: a project root is found by `git_root_within` here. A new
    /// walk that looks for `.git` on its own anywhere in the binary, the
    /// engine or memory fails until it asks this one (or is looked at and
    /// listed). Test code — a `#[cfg(test)]` module, or a `*tests.rs`
    /// file — makes fixtures and is not counted.
    #[test]
    fn the_engine_has_one_git_root_walk() {
        const LISTED: &[(&str, usize, &str)] = &[
            (
                "crates/linggen-engine/src/engine/tools/search_exec.rs",
                1,
                "the walk itself",
            ),
            (
                "crates/linggen-memory/src/memory/scope.rs",
                1,
                "memory's own walk; held equal by the_memory_root_walk_agrees_with_this_one",
            ),
            (
                "src/server/mcp_agent.rs",
                1,
                "is this a linked worktree (a .git file), not a root walk",
            ),
        ];
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut found: Vec<(String, usize)> = Vec::new();
        for dir in [
            "src",
            "crates/linggen-engine/src",
            "crates/linggen-memory/src",
        ] {
            for entry in WalkBuilder::new(root.join(dir)).build().flatten() {
                let path = entry.path();
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if !name.ends_with(".rs") || name.ends_with("tests.rs") {
                    continue;
                }
                let text = std::fs::read_to_string(path).unwrap();
                let n = without_test_modules(&text)
                    .matches("join(\".git\")")
                    .count();
                if n > 0 {
                    let rel = path.strip_prefix(&root).unwrap().to_string_lossy();
                    found.push((rel.to_string(), n));
                }
            }
        }
        found.sort();
        let mut want: Vec<(String, usize)> =
            LISTED.iter().map(|(f, n, _)| (f.to_string(), *n)).collect();
        want.sort();
        assert_eq!(
            found, want,
            "a project root found by a second walk — call find_git_root, or list the file"
        );
    }

    /// The memory crate finds a session's root by its own walk
    /// (`ling_mem::memory::scope::find_root`: it cannot call up into the
    /// engine). Both walks name the same root from every place, including a
    /// home given in one form and a folder in the other (`/var` vs
    /// `/private/var` on macOS).
    #[test]
    fn the_memory_root_walk_agrees_with_this_one() {
        let dir = tempfile::tempdir().unwrap();
        let outer = dir.path().canonicalize().unwrap();
        std::fs::create_dir(outer.join(".git")).unwrap();
        let home = outer.join("home");
        let repo = home.join("work/harbor");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("src/deep")).unwrap();
        std::fs::create_dir_all(home.join("work/chat")).unwrap();
        std::fs::create_dir(outer.join("elsewhere")).unwrap();
        // The same home as the caller may have it: not canonicalized.
        let home_as_given = dir.path().join("home");

        let places = [
            repo.join("src/deep"),
            repo.clone(),
            home.join("work/chat"),
            home.clone(),
            outer.join("elsewhere"),
        ];
        for home in [&home, &home_as_given] {
            for cwd in &places {
                let ours = git_root_within(cwd, Some(home)).unwrap_or_else(|| cwd.clone());
                let memory = ling_mem::memory::scope::find_root(cwd, home);
                assert_eq!(memory, ours, "cwd {cwd:?}, home {home:?}");
            }
        }
        // And with a dotfiles repo at home, home is still never a root.
        std::fs::create_dir(home.join(".git")).unwrap();
        let chat = home.join("work/chat");
        assert_eq!(ling_mem::memory::scope::find_root(&chat, &home), chat);
    }
}

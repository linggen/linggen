//! A scratch git repository with linked worktrees, for the tests of the
//! chain (here and in `own_memory`). Made with `git` itself, so the `.git`
//! files are the real ones.

use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A repository at `main` with one commit, and a linked worktree beside it
/// for each of `linked` (a folder name under `main`'s parent). Their paths.
pub(crate) fn repo_with_worktrees(main: &Path, linked: &[&str]) -> Vec<PathBuf> {
    std::fs::create_dir_all(main).unwrap();
    git(main, &["init", "-q"]);
    git(main, &["commit", "-q", "--allow-empty", "-m", "first"]);
    linked
        .iter()
        .map(|name| {
            let at = main.parent().unwrap().join(name);
            git(main, &["worktree", "add", "-q", "-b", name, at.to_str().unwrap()]);
            at
        })
        .collect()
}

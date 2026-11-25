//! How a session's folder maps to the places agents keep a project's notes:
//! the chain of folders a session stands in, and a project's folder name in
//! the one encoding a declaration may name (`dash`).

use super::disk::canonical;
use super::Env;
use std::path::{Path, PathBuf};

/// An agent's folder name for a project: every character that is not an
/// ASCII letter or digit becomes `-` (a character outside the BMP, two).
/// The mapping is lossy, so it is only ever applied forward: a folder is
/// found by encoding a path, never by decoding a name.
pub(crate) fn encode_project_path(path: &Path) -> String {
    let mut out = String::new();
    for c in path.to_string_lossy().chars() {
        match c.is_ascii_alphanumeric() {
            true => out.push(c),
            false => (0..c.len_utf16()).for_each(|_| out.push('-')),
        }
    }
    out
}

/// The folders a session at `cwd` is inside, nearest first: `cwd`, its
/// parents, up to but never including the home folder or `/` (a memory
/// there belongs to no project). The real path is tried too, as a host may
/// have resolved a link.
pub(crate) fn project_chain(env: &Env, cwd: &Path) -> Vec<PathBuf> {
    let mut chain: Vec<PathBuf> = Vec::new();
    let real = canonical(cwd);
    for start in std::iter::once(cwd.to_path_buf()).chain(real) {
        for dir in start.ancestors() {
            let stop = dir == env.home() || dir.parent().is_none();
            if stop {
                break;
            }
            if !chain.iter().any(|seen| seen == dir) {
                chain.push(dir.to_path_buf());
            }
        }
    }
    chain
}

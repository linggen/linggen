//! Roots whose reads require a specific grant, and engine secrets whose
//! reads require a local interactive owner.

use super::model::normalize_path;
use std::path::{Path, PathBuf};

const SECRET_FILES: &[&str] = &[
    "account.toml", "credentials.json", "codex_auth.json", "remote.toml",
    "paired-devices.json", "push/state.json",
];

const HOME_ROOTS: &[&str] = &[
    "Library/Application Support/Google/Chrome",
    "Library/Application Support/Chromium",
    "Library/Application Support/Firefox",
    "Library/Application Support/BraveSoftware",
    "Library/Application Support/Microsoft Edge",
    "Library/Safari", "Library/Mail", "Library/Messages", "Library/Keychains",
    "Library/Containers/com.apple.Safari", "Pictures", "Photos", "Mail", "Messages",
    ".config/google-chrome", ".config/chromium", ".mozilla", ".thunderbird",
    ".config/gcloud", ".config/gh", ".config/1Password", ".local/share/keyrings",
    ".kube", ".docker/config.json", ".git-credentials", ".netrc", ".npmrc",
    ".claude/projects", ".codex/sessions",
    "Library/Cookies", "Library/Group Containers",
    "Library/Application Support/1Password", "Library/Application Support/Arc",
    "Library/Application Support/com.apple.TCC", ".config/op",
    ".cargo/credentials.toml", ".cargo/credentials", ".pypirc", ".gem/credentials",
    ".terraform.d", ".zsh_history", ".bash_history", ".zsh_sessions",
];

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Protected {
    Place(PathBuf),
    Secret(PathBuf),
}

pub(super) fn resolved(path: &Path) -> PathBuf {
    PathBuf::from(normalize_path(&path.to_string_lossy()))
}

/// Existing paths use filesystem identity (including filesystem-specific
/// case and Unicode equivalence). Named policy components also match
/// without regard to case, even on a case-sensitive volume.
pub(super) fn same(a: &Path, b: &Path) -> bool {
    if a == b || a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()) {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(a), Ok(b)) = (a.metadata(), b.metadata()) {
            return (a.dev(), a.ino()) == (b.dev(), b.ino());
        }
    }
    false
}

pub(super) fn within(path: &Path, root: &Path) -> bool {
    path.starts_with(root) || path.ancestors().any(|p| same(p, root))
}

fn protected_component(component: &str) -> bool {
    let name = component.to_ascii_lowercase();
    matches!(name.as_str(), ".ssh" | ".gnupg" | ".aws" | ".azure" | ".env" | "keychains")
        || name.starts_with(".env.")
        || name.ends_with(".photoslibrary")
        || name.ends_with(".keychain")
        || name.ends_with(".keychain-db")
}

fn home_roots() -> impl Iterator<Item = PathBuf> {
    dirs::home_dir().into_iter().flat_map(|home| HOME_ROOTS.iter().map(move |name| home.join(name)))
}

fn engine_roots() -> [PathBuf; 1] {
    [crate::paths::linggen_home().clone()]
}

pub(super) fn protected(path: &Path) -> Option<Protected> {
    let real = resolved(path);
    for home in engine_roots() {
        for name in SECRET_FILES {
            let file = home.join(name);
            if same(path, &file) || same(&real, &resolved(&file)) {
                return Some(Protected::Secret(resolved(&file)));
            }
        }
    }
    for candidate in [path, real.as_path()] {
        for ancestor in candidate.ancestors() {
            if ancestor.file_name().and_then(|n| n.to_str()).is_some_and(protected_component) {
                return Some(Protected::Place(resolved(ancestor)));
            }
        }
        for root in engine_roots().into_iter().chain(home_roots()) {
            if within(candidate, &root) || within(candidate, &resolved(&root)) {
                return Some(Protected::Place(resolved(&root)));
            }
        }
        for parent in ["/Users", "/home", "/var/root", "/root"] {
            let parent = Path::new(parent);
            if let Some(root) = candidate.ancestors().find(|p| p.parent() == Some(parent)) {
                if dirs::home_dir().is_none_or(|home| !within(&home, root)) {
                    return Some(Protected::Place(root.to_path_buf()));
                }
            }
        }
    }
    None
}

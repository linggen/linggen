//! What the person declined this run: a denied permission prompt closes the
//! target's project to every tool for the rest of the run, so the model
//! cannot walk around the "no" with another tool (permission-spec.md
//! "Asking for permission"). Never persisted — the next run asks again.

use super::model::normalize_path;
use std::path::{Path, PathBuf};

/// The folders declined in the current run.
#[derive(Debug, Clone, Default)]
pub struct DeclinedScopes(Vec<PathBuf>);

impl DeclinedScopes {
    /// Forget them: a new run asks again.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Record a declined target; returns the scope it closes.
    pub fn decline(&mut self, target: &str) -> PathBuf {
        let scope = declined_scope(Path::new(&normalize_path(target)));
        if !self.0.contains(&scope) {
            self.0.push(scope.clone());
        }
        scope
    }

    /// The declined scope `target` falls inside, if any.
    pub fn covering(&self, target: &str) -> Option<&Path> {
        let target = PathBuf::from(normalize_path(target));
        self.0
            .iter()
            .find(|scope| target.starts_with(scope))
            .map(PathBuf::as_path)
    }
}

/// The scope a declined target closes: its git project root, else its
/// folder. Never the home folder or `/` — those close only the target.
fn declined_scope(target: &Path) -> PathBuf {
    let folder = if target.is_dir() {
        target
    } else {
        target.parent().unwrap_or(target)
    };
    let scope = crate::engine::tools::search_exec_find_git_root(folder)
        .unwrap_or_else(|| folder.to_path_buf());
    if too_wide(&scope) {
        return target.to_path_buf();
    }
    scope
}

fn too_wide(scope: &Path) -> bool {
    let home = dirs::home_dir().map(|h| PathBuf::from(normalize_path(&h.to_string_lossy())));
    scope.parent().is_none() || home.as_deref() == Some(scope)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_declined_repo_closes_to_every_path_inside_it() {
        let repo = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
        std::fs::create_dir_all(repo.path().join(".git")).unwrap();
        std::fs::create_dir_all(repo.path().join("src")).unwrap();
        let mut scopes = DeclinedScopes::default();
        let scope = scopes.decline(&repo.path().join("src").to_string_lossy());
        assert_eq!(
            scope,
            PathBuf::from(normalize_path(&repo.path().to_string_lossy()))
        );
        assert!(scopes
            .covering(&repo.path().join(".git/HEAD").to_string_lossy())
            .is_some());
        assert!(scopes
            .covering(&repo.path().join("README.md").to_string_lossy())
            .is_some());
        let outside = repo.path().parent().unwrap().join("elsewhere");
        assert!(scopes.covering(&outside.to_string_lossy()).is_none());
        scopes.clear();
        assert!(scopes
            .covering(&repo.path().join("src").to_string_lossy())
            .is_none());
    }

    #[test]
    fn without_a_repo_the_folder_is_declined_never_home() {
        let dir = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
        let file = dir.path().join("notes.txt");
        let mut scopes = DeclinedScopes::default();
        let scope = scopes.decline(&file.to_string_lossy());
        assert_eq!(
            scope,
            PathBuf::from(normalize_path(&dir.path().to_string_lossy()))
        );
        // A file straight in home closes only itself, not all of home.
        let mut home_scopes = DeclinedScopes::default();
        home_scopes.decline("~/declined-test-file.txt");
        assert!(home_scopes.covering("~/declined-test-file.txt").is_some());
        assert!(home_scopes.covering("~/other-file.txt").is_none());
    }
}

//! Where the previous binary is kept for rollback (`ling update`, `ling
//! restart`). Beside the binary as `<name>.prev` — except inside a macOS
//! `.app` bundle, whose sealed contents must not change: there it is kept
//! under `~/.linggen/prev/<encoded folder>/<name>.prev` (storage-spec.md).

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// The previous binary's place for `<dir>/<name>`. A copy an older engine
/// left inside a bundle is moved out on the way (or removed when one is
/// already kept outside), so the bundle's seal holds again.
pub fn prev_location(dir: &Path, name: &str) -> PathBuf {
    prev_location_in(crate::paths::linggen_home(), dir, name)
}

pub(crate) fn prev_location_in(home: &Path, dir: &Path, name: &str) -> PathBuf {
    let beside = dir.join(format!("{name}.prev"));
    if !in_app_bundle(dir) {
        return beside;
    }
    let kept = home
        .join("prev")
        .join(encode(dir))
        .join(format!("{name}.prev"));
    retire(&beside, &kept);
    kept
}

/// A path inside some `X.app` folder.
fn in_app_bundle(dir: &Path) -> bool {
    dir.components().any(|c| {
        Path::new(c.as_os_str())
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("app"))
    })
}

/// `/Applications/Linggen.app/Contents/MacOS` → `-Applications-Linggen.app-Contents-MacOS`,
/// as project paths are encoded.
fn encode(dir: &Path) -> String {
    dir.to_string_lossy().replace(['/', '\\'], "-")
}

fn retire(beside: &Path, kept: &Path) {
    if !beside.is_file() {
        return;
    }
    let result = if kept.is_file() {
        std::fs::remove_file(beside).map_err(anyhow::Error::from)
    } else {
        move_file(beside, kept)
    };
    if let Err(e) = result {
        tracing::warn!(
            "could not move {} out of the app bundle: {e:#}",
            beside.display()
        );
    }
}

/// Rename `from` to `to`, making `to`'s folder; across volumes, copy then
/// remove (the copy is a fresh file, never written over a live one).
pub fn move_file(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not make {}", parent.display()))?;
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    let _ = std::fs::remove_file(to);
    std::fs::copy(from, to)
        .with_context(|| format!("could not move {} to {}", from.display(), to.display()))?;
    std::fs::remove_file(from).with_context(|| format!("could not remove {}", from.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outside_a_bundle_the_previous_binary_stays_beside_it() {
        let home = tempfile::tempdir().unwrap();
        let dir = Path::new("/usr/local/bin");
        assert_eq!(
            prev_location_in(home.path(), dir, "ling"),
            dir.join("ling.prev")
        );
    }

    #[test]
    fn inside_a_bundle_it_is_kept_under_the_home_and_an_old_copy_moves_out() {
        let home = tempfile::tempdir().unwrap();
        let apps = tempfile::tempdir().unwrap();
        let dir = apps.path().join("Linggen.app/Contents/MacOS");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ling.prev"), "old").unwrap();

        let kept = prev_location_in(home.path(), &dir, "ling");
        assert!(kept.starts_with(home.path().join("prev")), "{kept:?}");
        assert!(!kept.starts_with(apps.path()));
        assert!(
            !dir.join("ling.prev").exists(),
            "the bundle still holds ling.prev"
        );
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "old");

        // One already kept outside wins; the copy inside is just removed.
        std::fs::write(dir.join("ling.prev"), "older").unwrap();
        assert_eq!(prev_location_in(home.path(), &dir, "ling"), kept);
        assert!(!dir.join("ling.prev").exists());
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "old");
    }
}

//! Where the previous binary is kept for rollback (`ling update`, `ling
//! restart`). Beside the binary as `<name>.prev` — except inside a macOS
//! `.app` bundle, whose sealed contents must not change: there it is kept
//! under `~/.linggen/prev/<encoded folder>/<name>.prev` (storage-spec.md).

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// The previous binary's place for `<dir>/<name>`. A copy an older engine
/// left inside a bundle is moved out on the way, so the bundle's seal holds
/// again. It replaces one already kept outside: only an older engine's
/// update writes it there, so it is the later of the two.
pub fn prev_location(dir: &Path, name: &str) -> PathBuf {
    prev_location_in(crate::paths::linggen_home(), dir, name)
}

pub(crate) fn prev_location_in(home: &Path, dir: &Path, name: &str) -> PathBuf {
    let beside = dir.join(format!("{name}.prev"));
    if !in_app_bundle(dir) {
        return beside;
    }
    // One folder per bundle however its path was spelled (update and
    // restart may reach the same binary by different paths).
    let real = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let kept = home
        .join("prev")
        .join(encode(&real))
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

/// A failure only warns: the update goes on, the old copy stays put.
fn retire(beside: &Path, kept: &Path) {
    if !beside.is_file() {
        return;
    }
    if let Err(e) = move_file(beside, kept) {
        tracing::warn!(
            "could not move {} out of the app bundle: {e:#}",
            beside.display()
        );
    }
}

/// Rename `from` to `to` (replacing it), making `to`'s folder; across
/// volumes, copy to a fresh file beside `to`, rename that over `to`, then
/// remove `from` — `to` is never missing or half written on the way.
pub fn move_file(from: &Path, to: &Path) -> Result<()> {
    let parent = to.parent().context("no folder to move into")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("could not make {}", parent.display()))?;
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    let name = to.file_name().unwrap_or_default().to_string_lossy();
    let part = parent.join(format!(".{name}.part-{}", std::process::id()));
    let copied = std::fs::copy(from, &part)
        .map_err(anyhow::Error::from)
        .and_then(|_| std::fs::rename(&part, to).map_err(anyhow::Error::from));
    if let Err(e) = copied {
        let _ = std::fs::remove_file(&part);
        return Err(e).context(format!(
            "could not move {} to {}",
            from.display(),
            to.display()
        ));
    }
    std::fs::remove_file(from).with_context(|| format!("could not remove {}", from.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundle_path_with_spaces_gets_its_own_folder() {
        let home = tempfile::tempdir().unwrap();
        let a = prev_location_in(
            home.path(),
            Path::new("/Applications/Linggen.app/Contents/MacOS"),
            "ling",
        );
        let b = prev_location_in(
            home.path(),
            Path::new("/Users/x/My Apps/Linggen.app/Contents/MacOS"),
            "ling",
        );
        assert_ne!(a, b);
        assert!(b.starts_with(home.path().join("prev")), "{b:?}");
        assert_eq!(b.file_name().unwrap(), "ling.prev");
    }

    #[cfg(unix)]
    #[test]
    fn a_bundle_that_cannot_be_changed_keeps_its_copy_and_the_update_goes_on() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let apps = tempfile::tempdir().unwrap();
        let dir = apps.path().join("Linggen.app/Contents/MacOS");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ling.prev"), "old").unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let kept = prev_location_in(home.path(), &dir, "ling");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(kept.starts_with(home.path()));
        assert_eq!(
            std::fs::read_to_string(dir.join("ling.prev")).unwrap(),
            "old"
        );
    }

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

        // A copy an older engine's update left in the bundle later is the
        // newer previous binary: it replaces the one kept outside.
        std::fs::write(dir.join("ling.prev"), "newer").unwrap();
        assert_eq!(prev_location_in(home.path(), &dir, "ling"), kept);
        assert!(!dir.join("ling.prev").exists());
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "newer");
    }
}

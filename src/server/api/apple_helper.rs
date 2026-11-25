//! `ling-hear`, the Swift helper that hears a clip with Apple's on-device
//! recognition (`apple_speech/`, built by `build.rs`), carried inside `ling`
//! and put on disk where it can run: one file in the runtime folder, named by
//! what it holds, so a new `ling` with a new helper never runs the old one
//! and a helper that is running is never overwritten.
//!
//! Only the helper itself is written here. The sound a person speaks never
//! comes near this file (`stt_apple.rs` hands it over on the helper's stdin).

use std::path::PathBuf;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

#[cfg(target_os = "macos")]
static HELPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ling-hear"));
/// Only a Mac build has one (`build.rs`).
#[cfg(not(target_os = "macos"))]
static HELPER: &[u8] = &[];

const PREFIX: &str = "ling-hear-";

/// The helper on disk, put there if it is not, the older ones removed.
pub(super) fn on_disk() -> Result<PathBuf> {
    anyhow::ensure!(!HELPER.is_empty(), "this build carries no hearing helper");
    let dir = crate::runtime::runtime_dir().join("bin");
    let name = format!("{PREFIX}{}", &hex(&Sha256::digest(HELPER))[..16]);
    let path = dir.join(&name);
    if std::fs::metadata(&path).is_ok_and(|m| m.len() == HELPER.len() as u64) {
        return Ok(path);
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    // A new file renamed into place: never a write over one that may be running.
    let part = dir.join(format!(".{name}.{}", std::process::id()));
    std::fs::write(&part, HELPER).context("write the hearing helper")?;
    set_runnable(&part)?;
    std::fs::rename(&part, &path).context("put the hearing helper in place")?;
    forget_older(&dir, &name);
    Ok(path)
}

#[cfg(unix)]
fn set_runnable(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).context("make the hearing helper runnable")
}

#[cfg(not(unix))]
fn set_runnable(_: &std::path::Path) -> Result<()> {
    Ok(())
}

fn forget_older(dir: &std::path::Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(PREFIX) && name != keep {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    /// The carried helper lands where `ling` can run it, once, and answers: asking where a language
    /// stands is the one call that cannot download anything.
    #[test]
    fn the_carried_helper_runs_from_the_runtime_folder() {
        let path = super::on_disk().unwrap();
        assert!(path.starts_with(crate::runtime::runtime_dir().join("bin")), "{}", path.display());
        assert_eq!(super::on_disk().unwrap(), path, "the same file the second time");
        let out = std::process::Command::new(&path).args(["state", "en"]).output().unwrap();
        let said: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert!(said["code"].is_string(), "{said}");
    }
}

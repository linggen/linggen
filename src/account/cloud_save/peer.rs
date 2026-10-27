//! A paired device's copy of a skill's declared save — the phone playing the
//! same game on its own, kept in step with this Mac over the WebRTC channel
//! (`/api/skill-save/{skill}`). The same files `cloud.save` names, in the same
//! bundle the account's copy uses (`files.rs`), under the same lock.
//!
//! The device does the deciding, as the engine does against linggen.dev: it
//! keeps the fingerprint it last agreed on and sends it back as the base of a
//! write. Here a write only lands on that base — a save that moved on since
//! answers [`PeerWrite::Stale`] with what it is now, never a silent overwrite.
//! When the device writes over changes made here (both sides moved; its copy
//! won), each file it replaces is first kept beside itself as
//! `<file>.conflict-<unix secs>`, exactly as a pull from the account does — and
//! only once the save has been left alone for [`AT_REST`]: a game in play on
//! this Mac is never written under the player.

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use super::{lock, SaveTarget, SYNC_LOCK};

/// How long the save must have stood still before a device's copy may replace
/// changes made here.
pub const AT_REST: Duration = Duration::from_secs(10 * 60);

/// The save as it is here, for a device.
#[derive(Debug, Clone, Serialize)]
pub struct PeerCopy {
    /// The files' fingerprint, as text; empty when there are no files.
    pub fingerprint: String,
    /// The bundle (`files.rs` `encode`); null when there are no files.
    pub body: Value,
    /// Seconds since a save file was last written here.
    pub rest_s: u64,
}

/// What a device's write did.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "done", rename_all = "snake_case")]
pub enum PeerWrite {
    Saved {
        fingerprint: String,
        /// Copies of changes made here that the write replaced.
        conflicts: Vec<PathBuf>,
    },
    /// The save moved on since the device's base: here it is now.
    Stale { now: PeerCopy },
    /// Changes made here would be replaced while the game may be in play.
    InPlay { rest_s: u64 },
}

fn fingerprint_of(t: &SaveTarget, files: &super::Files) -> String {
    if files.is_empty() {
        String::new()
    } else {
        t.layout.fingerprint(files).to_string()
    }
}

fn copy_of(t: &SaveTarget, files: &super::Files) -> PeerCopy {
    PeerCopy {
        fingerprint: fingerprint_of(t, files),
        body: if files.is_empty() {
            Value::Null
        } else {
            t.layout.encode(files)
        },
        rest_s: rest_secs(t, files),
    }
}

/// Seconds since the newest of the save's files was written.
fn rest_secs(t: &SaveTarget, files: &super::Files) -> u64 {
    let newest = files
        .keys()
        .filter_map(|rel| {
            std::fs::metadata(t.layout.root.join(rel))
                .ok()?
                .modified()
                .ok()
        })
        .max();
    newest
        .and_then(|m| SystemTime::now().duration_since(m).ok())
        .map(|d| d.as_secs())
        .unwrap_or(u64::MAX)
}

/// The save as it is now, read under its lock.
pub async fn read(t: &SaveTarget) -> Result<PeerCopy> {
    let _one = SYNC_LOCK.lock().await;
    let _held = lock::acquire(&t.file()).await?;
    Ok(copy_of(t, &t.layout.read()))
}

/// Write a device's copy over the save, if the save is still at `base`.
/// `over_changes`: the device is replacing changes made here (it won a
/// conflict), so they are kept as copies, and only once the save is at rest.
pub async fn write(
    t: &SaveTarget,
    base: &str,
    body: &Value,
    over_changes: bool,
) -> Result<PeerWrite> {
    let incoming = t.layout.decode(body)?;
    let _one = SYNC_LOCK.lock().await;
    let _held = lock::acquire(&t.file()).await?;
    let current = t.layout.read();
    if fingerprint_of(t, &current) != base {
        return Ok(PeerWrite::Stale {
            now: copy_of(t, &current),
        });
    }
    let rest_s = rest_secs(t, &current);
    if over_changes && !current.is_empty() && rest_s < AT_REST.as_secs() {
        return Ok(PeerWrite::InPlay { rest_s });
    }
    let conflicts = if over_changes {
        t.layout.keep_conflicts(&current, &incoming)?
    } else {
        Vec::new()
    };
    t.layout.write(&current, &incoming)?;
    Ok(PeerWrite::Saved {
        fingerprint: fingerprint_of(t, &incoming),
        conflicts,
    })
}

#[cfg(test)]
mod tests {
    use super::super::files::Layout;
    use super::*;

    fn target(root: &std::path::Path) -> SaveTarget {
        SaveTarget {
            skill: "g".into(),
            layout: Layout {
                root: root.to_path_buf(),
                entries: vec!["data/state.json".into(), "data/worlds".into()],
                one: false,
                skip: vec!["art".into()],
            },
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("peer-save-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("data")).unwrap();
        dir
    }

    fn body(state: &str) -> Value {
        serde_json::json!({ "bundle": 1, "files": { "data/state.json": { "text": state } } })
    }

    #[tokio::test]
    async fn an_empty_save_reads_empty_and_takes_a_first_copy() {
        let root = scratch("empty");
        let t = target(&root);
        let copy = read(&t).await.unwrap();
        assert_eq!(copy.fingerprint, "");
        assert!(copy.body.is_null());
        let done = write(&t, "", &body("{\"a\":1}"), false).await.unwrap();
        let PeerWrite::Saved {
            fingerprint,
            conflicts,
        } = done
        else {
            panic!("{done:?}")
        };
        assert!(conflicts.is_empty());
        assert_eq!(read(&t).await.unwrap().fingerprint, fingerprint);
        assert_eq!(
            std::fs::read_to_string(root.join("data/state.json")).unwrap(),
            "{\"a\":1}"
        );
    }

    #[tokio::test]
    async fn a_write_on_a_stale_base_changes_nothing_and_says_what_is_there() {
        let root = scratch("stale");
        let t = target(&root);
        std::fs::write(root.join("data/state.json"), "{\"mac\":2}").unwrap();
        let done = write(&t, "12345", &body("{\"phone\":1}"), false)
            .await
            .unwrap();
        let PeerWrite::Stale { now } = done else {
            panic!("{done:?}")
        };
        assert_eq!(now.fingerprint, read(&t).await.unwrap().fingerprint);
        assert_eq!(
            std::fs::read_to_string(root.join("data/state.json")).unwrap(),
            "{\"mac\":2}"
        );
    }

    #[tokio::test]
    async fn over_changes_here_waits_for_rest_then_keeps_a_copy() {
        let root = scratch("over");
        let t = target(&root);
        std::fs::write(root.join("data/state.json"), "{\"mac\":2}").unwrap();
        let base = read(&t).await.unwrap().fingerprint;
        // Just written: the game may be in play here.
        let done = write(&t, &base, &body("{\"phone\":3}"), true)
            .await
            .unwrap();
        assert!(matches!(done, PeerWrite::InPlay { .. }), "{done:?}");
        // A fast-forward (nothing changed here since the device's base) is not held back.
        let done = write(&t, &base, &body("{\"phone\":3}"), false)
            .await
            .unwrap();
        assert!(matches!(done, PeerWrite::Saved { ref conflicts, .. } if conflicts.is_empty()));
        // At rest: the device's copy goes in, and what it replaced is kept beside it.
        std::fs::write(root.join("data/state.json"), "{\"mac\":4}").unwrap();
        let old = SystemTime::now() - AT_REST - Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(root.join("data/state.json"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        let base = read(&t).await.unwrap().fingerprint;
        let done = write(&t, &base, &body("{\"phone\":5}"), true)
            .await
            .unwrap();
        let PeerWrite::Saved { conflicts, .. } = done else {
            panic!("{done:?}")
        };
        assert_eq!(conflicts.len(), 1);
        assert_eq!(
            std::fs::read_to_string(&conflicts[0]).unwrap(),
            "{\"mac\":4}"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("data/state.json")).unwrap(),
            "{\"phone\":5}"
        );
    }
}

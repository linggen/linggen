//! Watch `~/.linggen/quests/` and say which app's quest facts changed.
//!
//! Apps write `<app>.json` there (tmp + rename); a page that shows quests reads
//! them only when it loads, so a fact written while it is open — a workout the
//! phone just mirrored — sat unseen until a refresh. Each change becomes one
//! global `QuestsChanged { app }`, debounced so a burst (the tmp file, then the
//! rename) is announced once. The engine never reads the files: the stem is
//! the only thing that crosses.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast;

use super::state::ServerState;
use crate::engine::events::ServerEvent;

const DEBOUNCE: Duration = Duration::from_millis(800);

/// The app a changed path speaks for: `<app>.json`, never a temp or hidden
/// file (a writer's `.<app>.json.tmp` or `<app>.json.tmp` is not a fact yet).
fn quest_app(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    if name.starts_with('.') {
        return None;
    }
    let stem = name.strip_suffix(".json")?;
    if stem.is_empty() || stem.contains('.') {
        return None;
    }
    Some(stem.to_string())
}

pub(super) fn spawn(state: &Arc<ServerState>) {
    let dir = crate::paths::quests_dir();
    // Created, not waited for: an install with no app writing quests yet
    // still arms the watcher, so the first file written is announced too.
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!("[quests] cannot create {}: {e}", dir.display());
        return;
    }
    if let Err(e) = watch(dir, state.events_tx.clone(), DEBOUNCE) {
        tracing::warn!("[quests] watcher unavailable: {e}");
    }
}

/// Arm the watcher on `dir`; changes are sent on `events` as they settle.
fn watch(
    dir: PathBuf,
    events: broadcast::Sender<ServerEvent>,
    debounce: Duration,
) -> notify::Result<()> {
    use notify::Watcher;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(ev) = res else { return };
        if !(ev.kind.is_create() || ev.kind.is_modify() || ev.kind.is_remove()) {
            return;
        }
        for app in ev.paths.iter().filter_map(|p| quest_app(p)) {
            let _ = tx.send(app);
        }
    })?;
    watcher.watch(&dir, notify::RecursiveMode::NonRecursive)?;
    tracing::info!("[quests] watching {}", dir.display());
    tokio::spawn(async move {
        let _watcher = watcher; // lives as long as this loop
        while let Some(first) = rx.recv().await {
            let mut apps = BTreeSet::from([first]);
            while let Ok(Some(app)) = tokio::time::timeout(debounce, rx.recv()).await {
                apps.insert(app);
            }
            for app in apps {
                tracing::debug!("[quests] {app} changed");
                let _ = events.send(ServerEvent::QuestsChanged { app });
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_settled_app_files_count() {
        assert_eq!(
            quest_app(Path::new("/q/health.json")).as_deref(),
            Some("health")
        );
        assert_eq!(
            quest_app(Path::new("/q/apple-shifu.json")).as_deref(),
            Some("apple-shifu")
        );
        assert_eq!(quest_app(Path::new("/q/health.json.tmp")), None);
        assert_eq!(quest_app(Path::new("/q/.health.json.tmp")), None);
        assert_eq!(quest_app(Path::new("/q/.DS_Store")), None);
        assert_eq!(quest_app(Path::new("/q/notes.txt")), None);
    }

    #[tokio::test]
    async fn a_written_file_is_announced_once_per_app() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = broadcast::channel(16);
        watch(dir.path().to_path_buf(), tx, Duration::from_millis(200)).unwrap();
        // Written the way apps write: a temp file, then a rename, then again.
        let tmp = dir.path().join("health.json.tmp");
        std::fs::write(&tmp, "{}").unwrap();
        std::fs::rename(&tmp, dir.path().join("health.json")).unwrap();
        std::fs::write(dir.path().join("health.json"), "{\"a\":1}").unwrap();

        let got = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("announced")
            .unwrap();
        assert!(matches!(got, ServerEvent::QuestsChanged { ref app } if app == "health"));
        // The burst settled into that one event.
        let more = tokio::time::timeout(Duration::from_millis(600), rx.recv()).await;
        assert!(more.is_err(), "burst announced more than once: {more:?}");
    }
}

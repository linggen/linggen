use anyhow::Result;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use tokio::sync::mpsc;
use tracing::{error, info};

pub struct FileWatcher {
    watcher: RecommendedWatcher,
    receiver: mpsc::Receiver<notify::Result<Event>>,
}

impl FileWatcher {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let (tx, rx) = mpsc::channel(100);

        let watcher = RecommendedWatcher::new(
            move |res| {
                if let Err(e) = tx.blocking_send(res) {
                    error!("Failed to send watch event: {}", e);
                }
            },
            Config::default(),
        )?;

        let mut watcher = Self {
            watcher,
            receiver: rx,
        };

        watcher.watch(path)?;

        Ok(watcher)
    }

    pub fn watch(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        info!("Watching directory: {:?}", path);
        self.watcher.watch(path, RecursiveMode::Recursive)?;
        Ok(())
    }

    pub async fn next_event(&mut self) -> Option<notify::Result<Event>> {
        self.receiver.recv().await
    }
}

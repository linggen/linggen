use crate::{FileWalker, FileWatcher};
use anyhow::Result;
use std::path::PathBuf;
use tracing::info;

pub struct IngestionService {
    walker: FileWalker,
    watcher: Option<FileWatcher>,
}

impl IngestionService {
    pub fn new(root: PathBuf) -> Self {
        Self {
            walker: FileWalker::new(&root),
            watcher: None,
        }
    }

    pub async fn start_watching(&mut self, root: PathBuf) -> Result<()> {
        let watcher = FileWatcher::new(&root)?;
        self.watcher = Some(watcher);
        info!("Started watching {:?}", root);
        Ok(())
    }

    pub async fn scan(&self) -> Result<Vec<PathBuf>> {
        self.walker.walk()
    }
}

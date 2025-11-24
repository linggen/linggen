use anyhow::Result;
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};
use tracing::info;

pub struct FileWalker {
    root: PathBuf,
}

impl FileWalker {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    pub fn walk(&self) -> Result<Vec<PathBuf>> {
        info!("Scanning directory: {:?}", self.root);
        let mut files = Vec::new();

        let walker = WalkBuilder::new(&self.root)
            .hidden(true) // Ignore hidden files (like .git)
            .git_ignore(true) // Respect .gitignore
            .build();

        for result in walker {
            match result {
                Ok(entry) => {
                    if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                        let path = entry.path();

                        // explicit check to skip .git if hidden(false) is ever re-enabled
                        if path.components().any(|c| c.as_os_str() == ".git") {
                            continue;
                        }

                        files.push(path.to_path_buf());
                    }
                }
                Err(err) => {
                    tracing::warn!("Error walking directory: {}", err);
                }
            }
        }

        info!("Found {} files", files.len());
        Ok(files)
    }
}

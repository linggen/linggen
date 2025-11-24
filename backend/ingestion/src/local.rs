use anyhow::Result;
use async_trait::async_trait;
use ignore::WalkBuilder;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use rememberme_core::{Document, SourceType};
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::channel;
use uuid::Uuid;

use crate::Ingestor;

pub struct LocalIngestor {
    pub path: PathBuf,
}

impl LocalIngestor {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn watch(&self) -> Result<()> {
        // Basic watcher setup
        let (tx, _rx) = channel();
        let mut watcher = RecommendedWatcher::new(tx, Config::default())?;
        watcher.watch(&self.path, RecursiveMode::Recursive)?;

        // In a real implementation, we would return the watcher or spawn a task to handle _rx
        // For now, we just verify we can create it.
        Ok(())
    }
}

#[async_trait]
impl Ingestor for LocalIngestor {
    async fn ingest(&self) -> Result<Vec<Document>> {
        let mut documents = Vec::new();
        let walker = WalkBuilder::new(&self.path)
            .hidden(true) // Ignore hidden files (like .git) by default
            .git_ignore(true)
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

                        // Try to read as string
                        if let Ok(content) = fs::read_to_string(path) {
                            let relative_path = path
                                .strip_prefix(&self.path)
                                .unwrap_or(path)
                                .to_string_lossy()
                                .to_string();

                            let doc = Document {
                                id: Uuid::new_v4().to_string(),
                                source_type: SourceType::Local,
                                source_url: path.to_string_lossy().to_string(),
                                content,
                                metadata: serde_json::json!({
                                    "file_path": relative_path,
                                }),
                            };
                            documents.push(doc);
                        }
                    }
                }
                Err(err) => {
                    eprintln!("Error walking directory: {}", err);
                }
            }
        }

        Ok(documents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_local_ingestion() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let root = temp_dir.path();

        let file_path = root.join("test.txt");
        let mut file = File::create(&file_path)?;
        writeln!(file, "Hello, local world!")?;

        let ingestor = LocalIngestor::new(root.to_path_buf());
        let docs = ingestor.ingest().await?;

        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].content.trim(), "Hello, local world!");

        Ok(())
    }
}

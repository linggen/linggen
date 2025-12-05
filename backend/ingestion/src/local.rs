use anyhow::Result;
use async_trait::async_trait;
use glob::Pattern;
use ignore::WalkBuilder;
use linggen_core::{Document, SourceType};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc::channel;
use uuid::Uuid;

use crate::extract::extract_text;
use crate::Ingestor;

pub struct LocalIngestor {
    pub path: PathBuf,
    pub include_patterns: Vec<Pattern>,
    pub exclude_patterns: Vec<Pattern>,
}

impl LocalIngestor {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            include_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
        }
    }

    /// Create a new LocalIngestor with include/exclude glob patterns
    pub fn with_patterns(
        path: PathBuf,
        include_patterns: &[String],
        exclude_patterns: &[String],
    ) -> Self {
        let include = include_patterns
            .iter()
            .filter_map(|p| Pattern::new(p).ok())
            .collect();
        let exclude = exclude_patterns
            .iter()
            .filter_map(|p| Pattern::new(p).ok())
            .collect();

        Self {
            path,
            include_patterns: include,
            exclude_patterns: exclude,
        }
    }

    /// Check if a file path matches the include/exclude patterns
    fn matches_patterns(&self, relative_path: &str) -> bool {
        // If include patterns are specified, file must match at least one
        let include_match = if self.include_patterns.is_empty() {
            true // No include patterns = include all
        } else {
            self.include_patterns.iter().any(|p| {
                p.matches(relative_path)
                    || p.matches(relative_path.rsplit('/').next().unwrap_or(relative_path))
            })
        };

        // If exclude patterns are specified, file must NOT match any
        let exclude_match = self.exclude_patterns.iter().any(|p| {
            p.matches(relative_path)
                || p.matches(relative_path.rsplit('/').next().unwrap_or(relative_path))
        });

        include_match && !exclude_match
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
        let mut skipped_by_pattern = 0;

        let mut builder = WalkBuilder::new(&self.path);
        builder.hidden(true); // Ignore hidden files (like .git) by default
        builder.git_ignore(true);

        // Explicitly add .linggen/notes to be indexed
        let notes_path = self.path.join(".linggen").join("notes");
        if notes_path.exists() {
            builder.add(notes_path);
        }

        let walker = builder.build();

        for result in walker {
            match result {
                Ok(entry) => {
                    if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                        let path = entry.path();

                        // explicit check to skip .git if hidden(false) is ever re-enabled
                        if path.components().any(|c| c.as_os_str() == ".git") {
                            continue;
                        }

                        let relative_path = path
                            .strip_prefix(&self.path)
                            .unwrap_or(path)
                            .to_string_lossy()
                            .to_string();

                        // Apply include/exclude pattern filtering
                        if !self.matches_patterns(&relative_path) {
                            skipped_by_pattern += 1;
                            continue;
                        }

                        // Extract text from file (supports PDF, DOCX, and plain text)
                        if let Some(content) = extract_text(path) {
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
                    tracing::warn!("Error walking directory: {}", err);
                }
            }
        }

        if skipped_by_pattern > 0 {
            tracing::info!(
                "Skipped {} files due to include/exclude patterns",
                skipped_by_pattern
            );
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

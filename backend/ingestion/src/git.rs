use anyhow::{Context, Result};
use async_trait::async_trait;
use git2::Repository;
use ignore::WalkBuilder;
use rememberme_core::{Document, SourceType};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

use crate::Ingestor;

pub struct GitIngestor {
    pub url: String,
    pub branch: Option<String>,
    pub local_path: PathBuf,
}

impl GitIngestor {
    pub fn new(url: String, local_path: PathBuf) -> Self {
        Self {
            url,
            branch: None,
            local_path,
        }
    }

    pub fn with_branch(mut self, branch: String) -> Self {
        self.branch = Some(branch);
        self
    }

    fn clone_or_open(&self) -> Result<Repository> {
        if self.local_path.exists() {
            // Check if it's a valid repo
            match Repository::open(&self.local_path) {
                Ok(repo) => {
                    // Ideally we should pull here, but for now just return the repo
                    Ok(repo)
                }
                Err(_) => {
                    // Directory exists but not a repo, or corrupt.
                    // For safety, let's error out rather than blowing it away.
                    anyhow::bail!(
                        "Directory exists but is not a git repository: {:?}",
                        self.local_path
                    );
                }
            }
        } else {
            Repository::clone(&self.url, &self.local_path).context("Failed to clone repository")
        }
    }
}

#[async_trait]
impl Ingestor for GitIngestor {
    async fn ingest(&self) -> Result<Vec<Document>> {
        let _repo = self.clone_or_open()?;

        let mut documents = Vec::new();
        let walker = WalkBuilder::new(&self.local_path)
            .hidden(true) // Ignore hidden files (like .git)
            .git_ignore(true)
            .build();

        for result in walker {
            match result {
                Ok(entry) => {
                    if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                        let path = entry.path();

                        // Skip .git directory explicitly if ignore crate doesn't catch it for some reason
                        if path.components().any(|c| c.as_os_str() == ".git") {
                            continue;
                        }

                        // Try to read as string. If binary, skip.
                        if let Ok(content) = fs::read_to_string(path) {
                            let relative_path = path
                                .strip_prefix(&self.local_path)
                                .unwrap_or(path)
                                .to_string_lossy()
                                .to_string();

                            let doc = Document {
                                id: Uuid::new_v4().to_string(),
                                source_type: SourceType::Git,
                                source_url: format!("{}/blob/HEAD/{}", self.url, relative_path), // Rough approximation
                                content,
                                metadata: serde_json::json!({
                                    "file_path": relative_path,
                                    "repo_url": self.url,
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
    use std::path::Path;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_git_ingestion_local() -> Result<()> {
        // Setup a temp directory for the "remote" repo
        let temp_dir = TempDir::new()?;
        let repo_path = temp_dir.path();

        // Initialize a git repo
        let repo = Repository::init(repo_path)?;

        // Create a file
        let file_path = repo_path.join("test.txt");
        let mut file = File::create(&file_path)?;
        writeln!(file, "Hello, world!")?;

        // Commit it
        let mut index = repo.index()?;
        index.add_path(Path::new("test.txt"))?;
        let oid = index.write_tree()?;
        let signature = repo.signature()?;
        let tree = repo.find_tree(oid)?;
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "Initial commit",
            &tree,
            &[],
        )?;

        // Now try to ingest from this "remote" (which is actually local)
        // We need another temp dir for the clone destination
        let clone_dir = TempDir::new()?;
        let clone_path = clone_dir.path().join("cloned_repo");

        let ingestor = GitIngestor::new(repo_path.to_string_lossy().to_string(), clone_path);

        let docs = ingestor.ingest().await?;

        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].content.trim(), "Hello, world!");
        assert!(docs[0].metadata["file_path"]
            .as_str()
            .unwrap()
            .contains("test.txt"));

        Ok(())
    }
}

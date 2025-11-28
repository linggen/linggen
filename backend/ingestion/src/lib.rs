use anyhow::Result;
use async_trait::async_trait;
use rememberme_core::Document;

pub mod walker;
pub use walker::FileWalker;

pub mod watcher;
pub use watcher::FileWatcher;

pub mod service;
pub use service::IngestionService;

pub mod extract;
pub use extract::extract_text;

pub mod git;
pub use git::GitIngestor;

pub mod local;
pub use local::LocalIngestor;

pub mod web;
pub use web::WebIngestor;

#[async_trait]
pub trait Ingestor: Send + Sync {
    /// Ingests documents from the source.
    async fn ingest(&self) -> Result<Vec<Document>>;
}

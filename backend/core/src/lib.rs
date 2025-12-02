use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SourceType {
    Git,
    Local,
    Web,
    Uploads,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceConfig {
    pub id: String,
    pub name: String,
    pub source_type: SourceType,
    pub path: String, // URL or file path
    pub enabled: bool,
    // File pattern filters (glob patterns like "*.cs", "*.md")
    #[serde(default)]
    pub include_patterns: Vec<String>,
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
    // Cached stats from last successful indexing
    pub chunk_count: Option<usize>,
    pub file_count: Option<usize>,
    pub total_size_bytes: Option<usize>,
    // Track individual file sizes for uploads (filename -> size in bytes)
    #[serde(default)]
    pub file_sizes: std::collections::HashMap<String, usize>,
    // Last upload time for uploads sources (ISO 8601 timestamp)
    pub last_upload_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub source_type: SourceType,
    pub source_url: String,
    pub content: String,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// Unique identifier for this chunk (UUID for the chunk row itself)
    pub id: Uuid,
    /// ID of the source this chunk belongs to (matches `SourceConfig.id`, e.g. a repo or local folder)
    pub source_id: String,
    /// Logical document identifier within a source (e.g. file path or URL), shared by all chunks from the same file
    pub document_id: String,
    /// Raw text content of this chunk
    pub content: String,
    /// Optional embedding vector for this chunk (e.g. 384‑dim sentence transformer output)
    pub embedding: Option<Vec<f32>>,
    /// Arbitrary JSON metadata for this chunk (e.g. `file_path`, language, tags)
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexingJob {
    pub id: String,
    pub source_id: String,
    pub source_name: String,
    pub source_type: SourceType,
    pub status: JobStatus,
    pub started_at: String, // ISO 8601 timestamp
    pub finished_at: Option<String>,
    pub files_indexed: Option<usize>,
    pub chunks_created: Option<usize>,
    pub total_files: Option<usize>, // Total number of files to index
    pub total_size_bytes: Option<usize>, // Total size in bytes
    pub error: Option<String>,
}

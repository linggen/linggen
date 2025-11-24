use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SourceType {
    Git,
    Local,
    Web,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceConfig {
    pub id: String,
    pub name: String,
    pub source_type: SourceType,
    pub path: String, // URL or file path
    pub enabled: bool,
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
    pub id: Uuid,
    pub source_id: String,
    pub document_id: String,
    pub content: String,
    pub embedding: Option<Vec<f32>>,
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

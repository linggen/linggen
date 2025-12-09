use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// HTTP client wrapper for Linggen API
pub struct ApiClient {
    base_url: String,
    client: reqwest::Client,
}

impl ApiClient {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            client: reqwest::Client::new(),
        }
    }

    /// Check backend status
    pub async fn get_status(&self) -> Result<AppStatusResponse> {
        let url = format!("{}/api/status", self.base_url);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to connect to Linggen backend")?;

        response
            .json::<AppStatusResponse>()
            .await
            .context("Failed to parse status response")
    }

    /// List all sources
    pub async fn list_sources(&self) -> Result<ListSourcesResponse> {
        let url = format!("{}/api/resources", self.base_url);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to list sources")?;

        response
            .json::<ListSourcesResponse>()
            .await
            .context("Failed to parse sources response")
    }

    /// Create a new source
    pub async fn create_source(&self, req: CreateSourceRequest) -> Result<SourceResponse> {
        let url = format!("{}/api/resources", self.base_url);
        let response = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .context("Failed to create source")?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to create source: {}", error_text);
        }

        response
            .json::<SourceResponse>()
            .await
            .context("Failed to parse create source response")
    }

    /// Trigger indexing for a source
    pub async fn index_source(&self, source_id: &str, mode: &str) -> Result<IndexSourceResponse> {
        let url = format!("{}/api/index_source", self.base_url);
        let req = IndexSourceRequest {
            source_id: source_id.to_string(),
            mode: mode.to_string(),
        };

        let response = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .context("Failed to trigger indexing")?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to index source: {}", error_text);
        }

        response
            .json::<IndexSourceResponse>()
            .await
            .context("Failed to parse index response")
    }

    /// Get recent jobs
    pub async fn list_jobs(&self) -> Result<ListJobsResponse> {
        let url = format!("{}/api/jobs", self.base_url);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to list jobs")?;

        response
            .json::<ListJobsResponse>()
            .await
            .context("Failed to parse jobs response")
    }
}

// API request/response types

#[derive(Debug, Deserialize)]
pub struct AppStatusResponse {
    pub status: String,
    pub message: Option<String>,
    pub progress: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateSourceRequest {
    pub name: String,
    pub resource_type: String,
    pub path: String,
    #[serde(default)]
    pub include_patterns: Vec<String>,
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct SourceResponse {
    pub id: String,
    pub name: String,
    pub resource_type: String,
    pub path: String,
    pub enabled: bool,
    pub include_patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListSourcesResponse {
    pub resources: Vec<ResourceInfo>,
}

#[derive(Debug, Deserialize)]
pub struct ResourceInfo {
    pub id: String,
    pub name: String,
    pub resource_type: String,
    pub path: String,
    pub enabled: bool,
    #[allow(dead_code)]
    pub include_patterns: Vec<String>,
    #[allow(dead_code)]
    pub exclude_patterns: Vec<String>,
    #[allow(dead_code)]
    pub latest_job: Option<IndexingJob>,
    pub stats: Option<SourceStats>,
}

#[derive(Debug, Deserialize)]
pub struct SourceStats {
    pub chunk_count: usize,
    pub file_count: usize,
    #[allow(dead_code)]
    pub total_size_bytes: usize,
}

#[derive(Debug, Serialize)]
pub struct IndexSourceRequest {
    pub source_id: String,
    pub mode: String,
}

#[derive(Debug, Deserialize)]
pub struct IndexSourceResponse {
    pub job_id: String,
    pub files_indexed: usize,
    pub chunks_created: usize,
}

#[derive(Debug, Deserialize)]
pub struct ListJobsResponse {
    pub jobs: Vec<IndexingJob>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IndexingJob {
    pub id: String,
    pub source_id: String,
    pub source_name: String,
    pub source_type: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub files_indexed: Option<usize>,
    pub chunks_created: Option<usize>,
    pub total_files: Option<usize>,
    pub total_size_bytes: Option<usize>,
    pub error: Option<String>,
}

//! Linggen API Client
//!
//! HTTP client for communicating with the Linggen backend API.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{error, info};

/// Client for the Linggen backend API
pub struct LinggenApiClient {
    client: reqwest::Client,
    api_url: String,
}

impl LinggenApiClient {
    pub fn new(api_url: String, timeout_secs: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .expect("Failed to build HTTP client");

        Self { client, api_url }
    }

    /// Call the enhance API
    pub async fn enhance(
        &self,
        query: &str,
        strategy: Option<String>,
        source_id: Option<String>,
    ) -> Result<EnhanceResponse> {
        info!(
            ">>> API Request: POST /api/enhance | query={:?}, strategy={:?}, source_id={:?}",
            query, strategy, source_id
        );

        let resp = self
            .client
            .post(format!("{}/api/enhance", self.api_url))
            .json(&EnhanceRequest {
                query: query.to_string(),
                strategy,
                source_id,
            })
            .send()
            .await
            .context("Failed to send enhance request")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            error!("<<< API Error: status={}, body={}", status, body);
            anyhow::bail!("API request failed ({}): {}", status, body);
        }

        let response: EnhanceResponse = resp
            .json()
            .await
            .context("Failed to parse enhance response")?;

        info!(
            "<<< API Response: intent={}, chunks={}, preferences_applied={}",
            response.intent.intent_type(),
            response.context_chunks.len(),
            response.preferences_applied
        );

        Ok(response)
    }

    /// Get list of resources/sources
    pub async fn list_resources(&self) -> Result<ListResourcesResponse> {
        info!(">>> API Request: GET /api/resources");

        let resp = self
            .client
            .get(format!("{}/api/resources", self.api_url))
            .send()
            .await
            .context("Failed to send resources request")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            error!("<<< API Error: status={}, body={}", status, body);
            anyhow::bail!("API request failed ({}): {}", status, body);
        }

        let response: ListResourcesResponse = resp
            .json()
            .await
            .context("Failed to parse resources response")?;

        info!(
            "<<< API Response: {} sources found",
            response.resources.len()
        );

        Ok(response)
    }

    /// Get server status
    pub async fn get_status(&self) -> Result<StatusResponse> {
        info!(">>> API Request: GET /api/status");

        let resp = self
            .client
            .get(format!("{}/api/status", self.api_url))
            .send()
            .await
            .context("Failed to send status request")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            error!("<<< API Error: status={}, body={}", status, body);
            anyhow::bail!("API request failed ({}): {}", status, body);
        }

        let response: StatusResponse = resp
            .json()
            .await
            .context("Failed to parse status response")?;

        info!(
            "<<< API Response: status={}, message={:?}, progress={:?}",
            response.status, response.message, response.progress
        );

        Ok(response)
    }
}

// ============================================================================
// API Request/Response Types
// ============================================================================

#[derive(Serialize)]
pub struct EnhanceRequest {
    pub query: String,
    pub strategy: Option<String>,
    pub source_id: Option<String>,
}

/// Intent can be either a string or an object with intent_type and confidence
#[derive(Deserialize)]
#[serde(untagged)]
pub enum ApiIntent {
    Simple(String),
    Detailed {
        intent_type: String,
        confidence: f64,
    },
}

impl ApiIntent {
    pub fn intent_type(&self) -> &str {
        match self {
            ApiIntent::Simple(s) => s,
            ApiIntent::Detailed { intent_type, .. } => intent_type,
        }
    }

    pub fn confidence(&self) -> f64 {
        match self {
            ApiIntent::Simple(_) => 1.0,
            ApiIntent::Detailed { confidence, .. } => *confidence,
        }
    }
}

#[derive(Deserialize)]
pub struct ContextMeta {
    pub source_id: String,
    #[allow(dead_code)]
    pub document_id: String,
    pub file_path: String,
}

#[derive(Deserialize)]
pub struct EnhanceResponse {
    #[allow(dead_code)]
    pub original_query: String,
    pub enhanced_prompt: String,
    pub intent: ApiIntent,
    pub context_chunks: Vec<String>,
    #[serde(default)]
    pub context_metadata: Vec<ContextMeta>,
    pub preferences_applied: bool,
}

#[derive(Deserialize)]
pub struct SourceStats {
    pub chunk_count: i64,
    pub file_count: i64,
    pub total_size_bytes: i64,
}

#[derive(Deserialize)]
pub struct Resource {
    pub id: String,
    pub name: String,
    pub resource_type: String,
    pub path: String,
    pub enabled: bool,
    #[serde(default)]
    pub stats: Option<SourceStats>,
}

#[derive(Deserialize)]
pub struct ListResourcesResponse {
    pub resources: Vec<Resource>,
}

#[derive(Deserialize)]
pub struct StatusResponse {
    pub status: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub progress: Option<String>,
}

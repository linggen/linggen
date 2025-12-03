//! Analytics module for Linggen usage tracking
//!
//! Collects anonymous usage data to help improve the product.
//! No personal information or code content is ever sent.

use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use storage::MetadataStore;
use tokio::sync::OnceCell;
use tracing::{debug, warn};
use uuid::Uuid;

/// Analytics endpoint URL (Cloudflare Worker)
const ANALYTICS_ENDPOINT: &str = "https://linggen-analytics.liangatbc.workers.dev/track";

/// Timeout for analytics requests (don't block the app)
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Event types tracked
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    AppStarted,
    SourceAdded,
}

/// Platform identifier
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Macos,
    Windows,
    Linux,
    Unknown,
}

impl Platform {
    pub fn current() -> Self {
        #[cfg(target_os = "macos")]
        return Platform::Macos;
        #[cfg(target_os = "windows")]
        return Platform::Windows;
        #[cfg(target_os = "linux")]
        return Platform::Linux;
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        return Platform::Unknown;
    }
}

/// Source type for source_added events
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceType {
    Local,
    Git,
    Web,
    Uploads,
}

impl From<linggen_core::SourceType> for SourceType {
    fn from(st: linggen_core::SourceType) -> Self {
        match st {
            linggen_core::SourceType::Local => SourceType::Local,
            linggen_core::SourceType::Git => SourceType::Git,
            linggen_core::SourceType::Web => SourceType::Web,
            linggen_core::SourceType::Uploads => SourceType::Uploads,
        }
    }
}

/// Size bucket for project classification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SizeBucket {
    Small,  // < 100 files
    Medium, // 100 - 1000 files
    Large,  // 1000 - 10000 files
    Xlarge, // > 10000 files
}

impl SizeBucket {
    pub fn from_file_count(count: usize) -> Self {
        match count {
            0..=99 => SizeBucket::Small,
            100..=999 => SizeBucket::Medium,
            1000..=9999 => SizeBucket::Large,
            _ => SizeBucket::Xlarge,
        }
    }
}

/// Payload for source_added events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceAddedPayload {
    pub source_type: SourceType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bucket: Option<SizeBucket>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_count: Option<usize>,
}

/// Payload for app_started events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStartedPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_launch: Option<bool>,
}

/// Analytics event to send
#[derive(Debug, Clone, Serialize)]
struct AnalyticsEvent {
    installation_id: String,
    event_type: EventType,
    app_version: String,
    platform: Platform,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<serde_json::Value>,
}

/// Analytics client for sending events
pub struct AnalyticsClient {
    client: Client,
    installation_id: String,
    app_version: String,
    platform: Platform,
    enabled: bool,
}

/// Global analytics client instance
static ANALYTICS_CLIENT: OnceCell<Arc<AnalyticsClient>> = OnceCell::const_new();

impl AnalyticsClient {
    /// Create a new analytics client
    pub fn new(installation_id: String, enabled: bool) -> Self {
        let client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            installation_id,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            platform: Platform::current(),
            enabled,
        }
    }

    /// Initialize the global analytics client
    pub async fn initialize(metadata_store: &Arc<MetadataStore>) -> Result<Arc<AnalyticsClient>> {
        // Get or create installation_id
        let installation_id = match metadata_store.get_setting("installation_id")? {
            Some(id) => {
                debug!("Using existing installation_id: {}", id);
                id
            }
            None => {
                let id = Uuid::new_v4().to_string();
                metadata_store.set_setting("installation_id", &id)?;
                debug!("Generated new installation_id: {}", id);
                id
            }
        };

        // Check if analytics is enabled (default: true)
        let app_settings = metadata_store.get_app_settings().unwrap_or_default();
        let enabled = app_settings.analytics_enabled.unwrap_or(true);

        let client = Arc::new(AnalyticsClient::new(installation_id, enabled));

        // Store in global
        let _ = ANALYTICS_CLIENT.set(client.clone());

        Ok(client)
    }

    /// Get the global analytics client
    pub fn get() -> Option<Arc<AnalyticsClient>> {
        ANALYTICS_CLIENT.get().cloned()
    }

    /// Check if analytics is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Send an app_started event
    pub async fn track_app_started(&self, first_launch: bool) {
        if !self.enabled {
            debug!("Analytics disabled, skipping app_started event");
            return;
        }

        let payload = AppStartedPayload {
            first_launch: Some(first_launch),
        };

        self.send_event(
            EventType::AppStarted,
            Some(serde_json::to_value(payload).ok()),
        )
        .await;
    }

    /// Send a source_added event
    pub async fn track_source_added(
        &self,
        source_type: linggen_core::SourceType,
        file_count: Option<usize>,
    ) {
        if !self.enabled {
            debug!("Analytics disabled, skipping source_added event");
            return;
        }

        let size_bucket = file_count.map(SizeBucket::from_file_count);

        let payload = SourceAddedPayload {
            source_type: source_type.into(),
            size_bucket,
            file_count,
        };

        self.send_event(
            EventType::SourceAdded,
            Some(serde_json::to_value(payload).ok()),
        )
        .await;
    }

    /// Send an analytics event (non-blocking)
    async fn send_event(&self, event_type: EventType, payload: Option<Option<serde_json::Value>>) {
        let event = AnalyticsEvent {
            installation_id: self.installation_id.clone(),
            event_type,
            app_version: self.app_version.clone(),
            platform: self.platform.clone(),
            payload: payload.flatten(),
        };

        debug!("Sending analytics event: {:?}", event);

        // Send in background, don't wait
        let client = self.client.clone();
        tokio::spawn(async move {
            match client.post(ANALYTICS_ENDPOINT).json(&event).send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        debug!("Analytics event sent successfully");
                    } else {
                        debug!("Analytics event failed with status: {}", response.status());
                    }
                }
                Err(e) => {
                    // Don't log errors at warn level to avoid spamming logs
                    // when offline or endpoint is unavailable
                    debug!("Failed to send analytics event: {}", e);
                }
            }
        });
    }
}

/// Convenience function to track app started
pub async fn track_app_started(first_launch: bool) {
    if let Some(client) = AnalyticsClient::get() {
        client.track_app_started(first_launch).await;
    } else {
        warn!("Analytics client not initialized, skipping app_started event");
    }
}

/// Convenience function to track source added
pub async fn track_source_added(source_type: linggen_core::SourceType, file_count: Option<usize>) {
    if let Some(client) = AnalyticsClient::get() {
        client.track_source_added(source_type, file_count).await;
    } else {
        warn!("Analytics client not initialized, skipping source_added event");
    }
}

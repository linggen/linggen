//! Proxy room consumer — connect to a room owner's linggen as a model provider.
//!
//! When this linggen instance is a "linggen" consumer in a proxy room,
//! this module establishes a WebRTC connection to the owner's linggen,
//! discovers available models, and registers them as proxy providers.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use crate::cli::login::load_remote_config;
use crate::server::ServerState;

// ---------------------------------------------------------------------------
// Per-room connection tracking
// ---------------------------------------------------------------------------

/// Tracks active proxy room connections so we can disconnect per-room.
pub struct ProxyRoomConnections {
    rooms: RwLock<HashMap<String, ProxyRoomInfo>>,
    /// Serializes connect/disconnect operations to prevent TOCTOU races.
    connect_lock: Mutex<()>,
}

struct ProxyRoomInfo {
    room_name: String,
    owner_name: String,
    /// Proxy model IDs registered for this room (e.g. ["proxy:gpt-4o"]).
    model_ids: Vec<String>,
    /// The proxy client — kept alive so the WebRTC connection persists,
    /// and needed to re-register models after per-room disconnect.
    client: Arc<crate::agent_manager::proxy_provider::ProxyModelClient>,
}

/// Status of a single proxy room connection (returned by the API).
#[derive(serde::Serialize)]
pub struct ProxyConnectionStatus {
    pub instance_id: String,
    pub room_name: String,
    pub owner_name: String,
    pub models: Vec<String>,
}

impl ProxyRoomConnections {
    pub fn new() -> Self {
        Self {
            rooms: RwLock::new(HashMap::new()),
            connect_lock: Mutex::new(()),
        }
    }

    /// Record a successful connection.
    async fn insert(
        &self,
        instance_id: String,
        room_name: String,
        owner_name: String,
        model_ids: Vec<String>,
        client: Arc<crate::agent_manager::proxy_provider::ProxyModelClient>,
    ) {
        self.rooms.write().await.insert(instance_id, ProxyRoomInfo {
            room_name, owner_name, model_ids, client,
        });
    }

    /// Remove a connection, returning the model IDs to unregister.
    async fn remove(&self, instance_id: &str) -> Option<Vec<String>> {
        self.rooms.write().await.remove(instance_id).map(|info| info.model_ids)
    }

    /// Remove all connections, returning all model IDs to unregister.
    async fn remove_all(&self) -> Vec<String> {
        let mut map = self.rooms.write().await;
        let ids: Vec<String> = map.values().flat_map(|info| info.model_ids.clone()).collect();
        map.clear();
        ids
    }

    /// Check if a room is already connected.
    pub async fn is_connected(&self, instance_id: &str) -> bool {
        self.rooms.read().await.contains_key(instance_id)
    }

    /// List all active connections.
    pub async fn list(&self) -> Vec<ProxyConnectionStatus> {
        self.rooms.read().await.iter().map(|(id, info)| ProxyConnectionStatus {
            instance_id: id.clone(),
            room_name: info.room_name.clone(),
            owner_name: info.owner_name.clone(),
            models: info.model_ids.clone(),
        }).collect()
    }
}

// ---------------------------------------------------------------------------
// Connect / disconnect
// ---------------------------------------------------------------------------

/// Rebuild the ModelManager from current local models + all tracked proxy rooms.
/// This preserves the health tracker from the old manager.
async fn rebuild_model_manager(state: &ServerState) {
    let mut model_lock = state.manager.models.write().await;
    let old_mm = Arc::clone(&model_lock);

    // Start with only local (non-proxy) models
    let local_configs: Vec<_> = old_mm.list_models().iter()
        .filter(|c| c.provider != "proxy")
        .map(|c| (*c).clone())
        .collect();
    let mut new_mm = crate::agent_manager::models::ModelManager::new(local_configs);

    // Preserve health tracker state (rate-limit backoffs, etc.)
    new_mm.health = Arc::clone(&old_mm.health);

    // Re-register proxy models for all connected rooms
    let rooms = state.proxy_connections.rooms.read().await;
    for info in rooms.values() {
        new_mm.register_proxy_models(
            info.client.clone(),
            // Re-derive model info from the registered IDs — the client
            // already knows the models, we just need the configs back.
            // Simplest: store the original model JSON in ProxyRoomInfo.
            // For now, build minimal configs from the tracked IDs.
            info.model_ids.iter().map(|id| {
                let model_name = id.strip_prefix("proxy:").unwrap_or(id);
                serde_json::json!({
                    "id": model_name,
                    "model": model_name,
                    "supports_tools": true,
                })
            }).collect(),
            Some(info.owner_name.clone()).filter(|s| !s.is_empty()),
        );
    }
    drop(rooms);

    *model_lock = Arc::new(new_mm);
}

/// Join a proxy room and register the owner's models as proxy providers.
///
/// This:
/// 1. Connects to the owner's linggen via WebRTC relay
/// 2. Lists available models via `list_models` DC message
/// 3. Registers them in the ModelManager with "proxy:" prefix
/// 4. Tracks the connection for per-room disconnect
///
/// The proxy models appear alongside local models and can be used by
/// the agent engine transparently.
pub async fn connect_proxy_room(
    state: Arc<ServerState>,
    instance_id: &str,
    owner_name: Option<String>,
    room_name: Option<String>,
) -> anyhow::Result<()> {
    // Serialize connect/disconnect to prevent TOCTOU races.
    let _guard = state.proxy_connections.connect_lock.lock().await;

    // Check again under the lock
    if state.proxy_connections.is_connected(instance_id).await {
        info!("Already connected to proxy room (instance: {instance_id}), skipping");
        return Ok(());
    }

    let config = load_remote_config()
        .ok_or_else(|| anyhow::anyhow!("Not logged in to linggen.dev. Run `ling login` first."))?;

    info!("Connecting to proxy room (instance: {instance_id})");

    // Establish WebRTC connection to the room owner
    let conn = super::proxy_client::connect_to_room(
        &config.relay_url,
        instance_id,
        &config.api_token,
        Some(state.events_tx.clone()),
    ).await?;

    info!("WebRTC connection established to proxy room");

    // Create the proxy model client with request demuxing
    let proxy_client = Arc::new(
        crate::agent_manager::proxy_provider::ProxyModelClient::new(
            conn.request_tx,
            conn.response_rx,
        )
    );

    // Discover models available on the owner's linggen
    let models = proxy_client.list_models().await?;
    info!("Discovered {} models from proxy room", models.len());

    if models.is_empty() {
        warn!("No models available from proxy room");
        return Err(anyhow::anyhow!("No shared models available from this room"));
    }

    // Register proxy models via full rebuild
    let mut model_lock = state.manager.models.write().await;
    let old_mm = Arc::clone(&model_lock);

    let local_configs: Vec<_> = old_mm.list_models().iter()
        .filter(|c| c.provider != "proxy")
        .map(|c| (*c).clone())
        .collect();
    let mut new_mm = crate::agent_manager::models::ModelManager::new(local_configs);
    new_mm.health = Arc::clone(&old_mm.health);

    // Re-register existing proxy rooms
    {
        let rooms = state.proxy_connections.rooms.read().await;
        for info in rooms.values() {
            new_mm.register_proxy_models(
                info.client.clone(),
                info.model_ids.iter().map(|id| {
                    let model_name = id.strip_prefix("proxy:").unwrap_or(id);
                    serde_json::json!({ "id": model_name, "model": model_name, "supports_tools": true })
                }).collect(),
                Some(info.owner_name.clone()).filter(|s| !s.is_empty()),
            );
        }
    }

    // Register new room's models
    let registered_ids = new_mm.register_proxy_models(proxy_client.clone(), models, owner_name.clone());

    *model_lock = Arc::new(new_mm);
    drop(model_lock);

    // Track the connection
    state.proxy_connections.insert(
        instance_id.to_string(),
        room_name.unwrap_or_default(),
        owner_name.unwrap_or_default(),
        registered_ids,
        proxy_client,
    ).await;

    info!("Proxy room models registered successfully");
    Ok(())
}

/// Disconnect a specific proxy room — remove only that room's models.
pub async fn disconnect_proxy_room_by_instance(state: Arc<ServerState>, instance_id: &str) {
    let _guard = state.proxy_connections.connect_lock.lock().await;

    match state.proxy_connections.remove(instance_id).await {
        Some(_) => {
            // Rebuild manager with remaining rooms
            rebuild_model_manager(&state).await;
            info!("Proxy room models removed (instance: {instance_id})");
        }
        None => {
            info!("No proxy connection found for instance {instance_id}");
        }
    }
}

/// Disconnect all proxy rooms — remove all proxy models.
pub async fn disconnect_all_proxy_rooms(state: Arc<ServerState>) {
    let _guard = state.proxy_connections.connect_lock.lock().await;

    let _model_ids = state.proxy_connections.remove_all().await;

    let mut model_lock = state.manager.models.write().await;
    let old_mm = Arc::clone(&model_lock);

    // Rebuild ModelManager with only non-proxy models, preserving health tracker
    let configs: Vec<_> = old_mm.list_models().iter()
        .filter(|c| c.provider != "proxy")
        .map(|c| (*c).clone())
        .collect();
    let mut new_mm = crate::agent_manager::models::ModelManager::new(configs);
    new_mm.health = Arc::clone(&old_mm.health);
    *model_lock = Arc::new(new_mm);

    info!("All proxy room models removed");
}

// ---------------------------------------------------------------------------
// Auto-connect on startup
// ---------------------------------------------------------------------------

/// Fetch joined rooms from linggen.dev and auto-connect to online rooms
/// where consumer_type is "linggen".
pub async fn auto_connect_joined_rooms(state: Arc<ServerState>) {
    let config = match load_remote_config() {
        Some(c) => c,
        None => return, // Not logged in, skip
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    let resp = match client
        .get(format!("{}/api/rooms/joined", config.relay_url))
        .bearer_auth(&config.api_token)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            warn!("Failed to fetch joined rooms: HTTP {}", r.status());
            return;
        }
        Err(e) => {
            warn!("Failed to fetch joined rooms: {e}");
            return;
        }
    };

    let body: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            warn!("Failed to parse joined rooms response: {e}");
            return;
        }
    };

    let rooms = match body.get("rooms").and_then(|r| r.as_array()) {
        Some(arr) => arr,
        None => return,
    };

    let mut connected = 0u32;
    for room in rooms {
        let consumer_type = room.get("consumer_type").and_then(|v| v.as_str()).unwrap_or("");
        let online = room.get("online").and_then(|v| v.as_bool()).unwrap_or(false);

        if consumer_type != "linggen" || !online {
            continue;
        }

        let instance_id = match room.get("instance_id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => continue,
        };
        let owner_name = room.get("owner_name").and_then(|v| v.as_str()).map(String::from);
        let room_name = room.get("name").and_then(|v| v.as_str()).map(String::from);

        match connect_proxy_room(state.clone(), &instance_id, owner_name, room_name).await {
            Ok(()) => connected += 1,
            Err(e) => warn!("Failed to auto-connect to proxy room {instance_id}: {e}"),
        }
    }

    if connected > 0 {
        info!("Auto-connected to {connected} proxy room(s)");
    }
}

//! User-tunable memory config — small JSON file at
//! `<data_dir>/memory/.config.json`. Read and written through
//! `GET`/`PUT /api/config` (`http::config`); the dream pass reads it too.
//!
//! Kept minimal on purpose. Schema lives here, not in a separate spec —
//! add a field, version-bump the default, ship.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::ops::network::Network;

/// User-tunable knobs persisted to disk. Add a field with a sensible
/// default and the existing config files round-trip cleanly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Short-term retention length: how long an episodic row lives
    /// before the forget sweep may evict it — and only on days a
    /// remember pass has already judged (`remembered_at` stamped).
    /// This is the human-facing knob the console / dream skill reads.
    #[serde(default = "default_episodic_ttl_days")]
    pub episodic_ttl_days: u32,

    /// Store-wide cosine floor for `search`. Applied server-side when a
    /// search request omits `min_score`, so every host (engine auto-recall,
    /// the CC/Codex recall hook, the CLI) shares one recall selectivity
    /// instead of each hardcoding its own. An explicit per-call `min_score`
    /// still overrides. Same role as `episodic_ttl_days`: a store property
    /// the daemon owns and clients defer to.
    #[serde(default = "default_recall_min_score")]
    pub recall_min_score: f32,

    /// Saved network choice; hosted memory listens where the engine does.
    /// `PUT /api/config` preserves the value on disk.
    #[serde(default)]
    pub network: Network,
}

fn default_episodic_ttl_days() -> u32 {
    crate::memory::expiry::DEFAULT_TTL_DAYS
}

fn default_recall_min_score() -> f32 {
    0.7
}

impl Default for Config {
    fn default() -> Self {
        Self {
            episodic_ttl_days: default_episodic_ttl_days(),
            recall_min_score: default_recall_min_score(),
            network: Network::default(),
        }
    }
}

pub(crate) fn config_path(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("memory").join(".config.json")
}

/// Read-or-default. Never errors on missing/corrupt — the store
/// always has a working config even on first run.
pub(crate) async fn load(data_dir: &std::path::Path) -> Config {
    let path = config_path(data_dir);
    let Ok(bytes) = tokio::fs::read(&path).await else {
        return Config::default();
    };
    serde_json::from_slice::<Config>(&bytes).unwrap_or_default()
}

pub(crate) async fn save(data_dir: &std::path::Path, cfg: &Config) -> Result<(), std::io::Error> {
    let path = config_path(data_dir);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    // Atomic-ish: write to .tmp + rename. Console writes are rare; no
    // need for a heavier lock.
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(cfg).expect("Config serializes");
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(tmp, path).await?;
    Ok(())
}

pub async fn get_config(
    state: &crate::handle::Memory,
) -> Result<serde_json::Value, crate::error::MemError> {
    let cfg = load(&state.data_dir).await;
    Ok(serde_json::to_value(cfg).expect("Config serializes"))
}

pub async fn put_config(
    state: &crate::handle::Memory,
    mut cfg: Config,
) -> Result<serde_json::Value, crate::error::MemError> {
    use crate::error::MemError;
    // The network choice is not this route's to change — see `network.rs`.
    cfg.network = load(&state.data_dir).await.network;
    if !crate::memory::expiry::TTL_DAYS_RANGE.contains(&cfg.episodic_ttl_days) {
        return Err(MemError::bad_request(
            "episodic_ttl_days must be from 1 to 365",
        ));
    }
    if !(0.0..=1.0).contains(&cfg.recall_min_score) {
        return Err(MemError::bad_request(
            "recall_min_score must be between 0.0 and 1.0",
        ));
    }
    save(&state.data_dir, &cfg)
        .await
        .map_err(|e| MemError::internal(anyhow::anyhow!(e)))?;
    Ok(serde_json::to_value(cfg).expect("Config serializes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_config_route_never_changes_the_network() {
        let dir = tempfile::tempdir().unwrap();
        let state =
            crate::handle::scratch_state(dir.path(), std::net::SocketAddr::from(([0, 0, 0, 0], 0)))
                .await;
        let cfg = Config {
            network: Network::Local,
            ..Config::default()
        };
        save(dir.path(), &cfg).await.unwrap();
        let sent = Config {
            network: Network::Lan,
            episodic_ttl_days: 9,
            ..Config::default()
        };
        put_config(&state, sent).await.unwrap();
        let stored = load(dir.path()).await;
        assert_eq!(stored.network, Network::Local);
        assert_eq!(stored.episodic_ttl_days, 9);
    }

    #[tokio::test]
    async fn an_old_config_file_reads_as_shared_on_the_lan() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"episodic_ttl_days":7}"#).unwrap();
        assert_eq!(load(dir.path()).await.network, Network::Lan);
    }
}

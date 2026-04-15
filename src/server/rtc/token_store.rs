//! Persistent token usage tracking for proxy room budget enforcement.
//!
//! Stores daily token counts at `~/.linggen/token_usage.json`.
//! Auto-resets when the date changes (UTC).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::info;

#[derive(Debug, Serialize, Deserialize)]
struct TokenUsageFile {
    /// UTC date string, e.g. "2026-04-15"
    date: String,
    /// Room-level total tokens used today (all consumers combined)
    room_total: i64,
    /// Per-consumer tokens used today. Key = user_id
    consumers: HashMap<String, i64>,
}

pub struct TokenUsageStore {
    data: TokenUsageFile,
    dirty: bool,
}

impl TokenUsageStore {
    /// Load from disk, or create zeroed if missing/stale.
    pub fn load() -> Self {
        let path = usage_path();
        let data = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<TokenUsageFile>(&s).ok())
            .unwrap_or_else(|| TokenUsageFile {
                date: today(),
                room_total: 0,
                consumers: HashMap::new(),
            });
        let mut store = Self { data, dirty: false };
        store.maybe_reset();
        store
    }

    /// Save to disk if dirty.
    pub fn flush(&mut self) {
        if !self.dirty { return; }
        let path = usage_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&self.data) {
            if std::fs::write(&path, json).is_ok() {
                self.dirty = false;
            }
        }
    }

    /// Check if a request is within budget. Returns true if allowed.
    pub fn check_budget(
        &mut self,
        user_id: &str,
        room_budget: Option<i64>,
        consumer_budget: Option<i64>,
    ) -> bool {
        self.maybe_reset();
        if let Some(rb) = room_budget {
            if self.data.room_total >= rb { return false; }
        }
        if let Some(cb) = consumer_budget {
            let used = self.data.consumers.get(user_id).copied().unwrap_or(0);
            if used >= cb { return false; }
        }
        true
    }

    /// Record token usage for a consumer.
    pub fn record_usage(&mut self, user_id: &str, tokens: i64) {
        self.maybe_reset();
        self.data.room_total += tokens;
        *self.data.consumers.entry(user_id.to_string()).or_insert(0) += tokens;
        self.dirty = true;
    }

    /// Get usage stats: (consumer_used, room_total).
    pub fn get_usage(&self, user_id: &str) -> (i64, i64) {
        let consumer = self.data.consumers.get(user_id).copied().unwrap_or(0);
        (consumer, self.data.room_total)
    }

    /// Reset counters if the date has changed.
    fn maybe_reset(&mut self) {
        let now = today();
        if self.data.date != now {
            info!("Token usage daily reset (was {})", self.data.date);
            self.data.date = now;
            self.data.room_total = 0;
            self.data.consumers.clear();
            self.dirty = true;
        }
    }
}

fn usage_path() -> std::path::PathBuf {
    crate::paths::linggen_home().join("token_usage.json")
}

fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

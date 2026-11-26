//! Linggen Cloud's chat models: what the site's price table holds, read from
//! `GET {site}/api/cloud/models` (`{"updated","starter","models":[{"id","label"}]}`).
//!
//! The engine names no Linggen Cloud model of its own: the last good answer
//! is kept in `~/.linggen/cache/cloud-models.json` for offline, and an engine
//! that has never read one knows only the first-run starter
//! ([`super::LINGGEN_CLOUD_MODEL_ID`]). Ticking is unchanged: a ticked model is
//! an ordinary row (`cloud_tick`).

use serde::{Deserialize, Serialize};
use std::sync::{LazyLock, RwLock};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloudModel {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloudModels {
    #[serde(default)]
    pub updated: String,
    pub starter: String,
    pub models: Vec<CloudModel>,
}

impl CloudModels {
    /// What a first run with nothing kept knows: the starter alone.
    fn first_run() -> Self {
        let id = super::LINGGEN_CLOUD_MODEL_ID.to_string();
        Self { updated: String::new(), starter: id.clone(), models: vec![CloudModel { label: id.clone(), id }] }
    }

    /// An answer worth keeping: some models, each with an id, and a starter among them.
    fn checked(self) -> Option<Self> {
        let ok = !self.models.is_empty()
            && self.models.iter().all(|m| !m.id.trim().is_empty())
            && self.models.iter().any(|m| m.id == self.starter);
        ok.then_some(self)
    }

    pub fn label_of(&self, id: &str) -> Option<&str> {
        self.models.iter().find(|m| m.id == id).map(|m| m.label.as_str())
    }
}

static CURRENT: LazyLock<RwLock<Option<CloudModels>>> = LazyLock::new(|| RwLock::new(None));

fn kept_path() -> std::path::PathBuf {
    crate::paths::linggen_home().join("cache/cloud-models.json")
}

fn read_kept() -> Option<CloudModels> {
    let text = std::fs::read_to_string(kept_path()).ok()?;
    serde_json::from_str::<CloudModels>(&text).ok()?.checked()
}

/// The models in force: the last good answer, else the first-run starter.
pub fn current() -> CloudModels {
    if let Some(held) = CURRENT.read().unwrap_or_else(|e| e.into_inner()).clone() {
        return held;
    }
    let read = read_kept().unwrap_or_else(CloudModels::first_run);
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = Some(read.clone());
    read
}

/// Take a body the site answered; keep it when it is good. True when it changed.
pub fn accept(body: &[u8]) -> bool {
    let Some(next) = serde_json::from_slice::<CloudModels>(body).ok().and_then(CloudModels::checked) else {
        return false;
    };
    if current() == next {
        return false;
    }
    let path = kept_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_vec(&next) {
        let _ = crate::state_fs::devices::write_atomic(&path, &text);
    }
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = Some(next);
    true
}

/// Ask the site once. True when the list changed. Never on a turn's path.
pub async fn refresh() -> bool {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();
    let url = format!("{}/api/cloud/models", crate::account::site_url());
    let Ok(resp) = client.get(url).send().await else { return false };
    if !resp.status().is_success() {
        return false;
    }
    match resp.bytes().await {
        Ok(body) => accept(&body),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_good_answer_is_checked_and_a_bad_one_is_not_kept() {
        let good = serde_json::json!({"updated": "2026-11-26", "starter": "a", "models": [{"id": "a", "label": "A"}, {"id": "b", "label": "B"}]});
        let models: CloudModels = serde_json::from_value(good).unwrap();
        assert_eq!(models.clone().checked().unwrap().label_of("b"), Some("B"));
        let no_starter = CloudModels { starter: "z".into(), ..models.clone() };
        assert!(no_starter.checked().is_none());
        let empty = CloudModels { models: vec![], ..models };
        assert!(empty.checked().is_none());
        assert!(!accept(b"not json"));
    }

    #[test]
    fn a_first_run_knows_only_the_starter() {
        let first = CloudModels::first_run();
        assert_eq!(first.models.len(), 1);
        assert_eq!(first.starter, super::super::LINGGEN_CLOUD_MODEL_ID);
    }
}

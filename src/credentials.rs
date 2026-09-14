use crate::config::ModelConfig;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Persisted format: ~/.linggen/credentials.json (version 2)
//
//   {
//     "version": 2,
//     "endpoints": [ { "provider": "gemini", "url": "https://…/v1beta/openai", "api_key": "…" } ],
//     "models":    { "<model id>": { "api_key" } },      // rare per-model overrides
//     "services":  { "tavily": { "api_key" } }           // keys that are not a model's
//   }
//
// A key belongs to an account at an endpoint (provider + base URL), not to a
// model id: every model on the endpoint shares it, adding a model needs no
// second paste, and deleting a model orphans nothing. Version 1 was a flat
// map keyed by model id; it migrates on the first load that knows the
// configured models (see `load_for`).
// ---------------------------------------------------------------------------

pub const CREDENTIALS_VERSION: u32 = 2;

#[derive(Debug, Serialize, Deserialize, Default, Clone, PartialEq)]
pub struct KeyEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone, PartialEq)]
pub struct EndpointKey {
    pub provider: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Credentials {
    #[serde(default = "current_version")]
    pub version: u32,
    /// One entry per endpoint. Read as a list, or as the map keyed by
    /// `endpoint_id` an earlier build wrote; always written as a list.
    #[serde(default, deserialize_with = "endpoints_list_or_map")]
    pub endpoints: Vec<EndpointKey>,
    #[serde(default)]
    pub models: BTreeMap<String, KeyEntry>,
    #[serde(default)]
    pub services: BTreeMap<String, KeyEntry>,
}

fn current_version() -> u32 {
    CREDENTIALS_VERSION
}

/// The endpoints section as a list, or as the `{ "<id>": {…} }` map an
/// earlier version-2 build wrote (ids in sorted order).
fn endpoints_list_or_map<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<EndpointKey>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Shape {
        List(Vec<EndpointKey>),
        Map(BTreeMap<String, EndpointKey>),
    }
    Ok(match Shape::deserialize(d)? {
        Shape::List(v) => v,
        Shape::Map(m) => m.into_values().collect(),
    })
}

impl Default for Credentials {
    fn default() -> Self {
        Self {
            version: CREDENTIALS_VERSION,
            endpoints: Vec::new(),
            models: BTreeMap::new(),
            services: BTreeMap::new(),
        }
    }
}

/// Two endpoints are the same when provider and base URL match, case and
/// trailing slashes aside.
pub fn same_endpoint(provider_a: &str, url_a: &str, provider_b: &str, url_b: &str) -> bool {
    provider_a.trim().eq_ignore_ascii_case(provider_b.trim())
        && url_a
            .trim()
            .trim_end_matches('/')
            .eq_ignore_ascii_case(url_b.trim().trim_end_matches('/'))
}

/// The version-1 row shape: a key entered for one model id, optionally
/// stamped with the endpoint it was entered for.
#[derive(Debug, Deserialize, Default, Clone)]
struct LegacyEntry {
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

impl Credentials {
    /// Load from disk. A version-1 file migrates in memory using only the
    /// stamps it carries; rows without a stamp that name no configured model
    /// are kept under `services` so nothing disappears. Prefer `load_for`
    /// when the configured models are known — it also writes the migrated
    /// file back.
    pub fn load(file: &Path) -> Self {
        Self::load_with(file, &[])
    }

    /// Load, migrating a version-1 file against the configured models and
    /// saving the result (the old file stays beside it as `.v1.bak`).
    pub fn load_for(file: &Path, configured: &[ModelConfig]) -> Self {
        let (creds, migrated) = Self::load_inner(file, configured);
        if migrated {
            let bak = file.with_extension("json.v1.bak");
            if !bak.exists() {
                if let Err(e) = std::fs::copy(file, &bak) {
                    warn!("credentials: could not keep the earlier copy at {}: {e}", bak.display());
                }
            }
            match creds.save(file) {
                Ok(()) => info!(
                    "credentials: migrated to version {} — {} endpoint key(s), {} model override(s), {} service key(s)",
                    CREDENTIALS_VERSION, creds.endpoints.len(), creds.models.len(), creds.services.len()
                ),
                Err(e) => warn!("credentials: migrated in memory but could not save: {e}"),
            }
        }
        creds
    }

    fn load_with(file: &Path, configured: &[ModelConfig]) -> Self {
        Self::load_inner(file, configured).0
    }

    /// Returns the credentials and whether they came from a version-1 file.
    fn load_inner(file: &Path, configured: &[ModelConfig]) -> (Self, bool) {
        if !file.exists() {
            return (Self::default(), false);
        }
        let content = match std::fs::read_to_string(file) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to read credentials.json: {}", e);
                return (Self::default(), false);
            }
        };
        let value: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                warn!("Failed to parse credentials.json: {}", e);
                return (Self::default(), false);
            }
        };
        let version = value.get("version").and_then(|v| v.as_u64()).unwrap_or(1);
        if version >= 2 {
            let map_shaped = value.get("endpoints").map(|e| e.is_object()).unwrap_or(false);
            return match serde_json::from_value::<Credentials>(value) {
                Ok(c) => (c, map_shaped),
                Err(e) => {
                    warn!("Failed to parse credentials.json: {}", e);
                    (Self::default(), false)
                }
            };
        }
        let rows: BTreeMap<String, LegacyEntry> = match serde_json::from_value(value) {
            Ok(r) => r,
            Err(e) => {
                warn!("Failed to parse credentials.json (v1): {}", e);
                return (Self::default(), false);
            }
        };
        (Self::migrate_v1(rows, configured), true)
    }

    /// Version 1 → 2. Precedence for an endpoint's key: a row whose model is
    /// still configured, then a stamped orphan, ids in sorted order. A
    /// configured model whose key differs from its endpoint's becomes an
    /// override. An unstamped row naming no configured model has no known
    /// endpoint and is kept as a service key rather than guessed.
    fn migrate_v1(rows: BTreeMap<String, LegacyEntry>, configured: &[ModelConfig]) -> Self {
        let mut out = Self::default();
        let key_of = |e: &LegacyEntry| e.api_key.clone().filter(|k| !k.is_empty());
        // Pass 1: rows of configured models set their endpoint's key.
        for (id, row) in &rows {
            let Some(key) = key_of(row) else { continue };
            if let Some(m) = configured.iter().find(|m| &m.id == id) {
                match out.endpoint_key(&m.provider, &m.url) {
                    None => out.set_endpoint_key(&m.provider, &m.url, Some(key)),
                    Some(k) if k != key => {
                        out.models.insert(id.clone(), KeyEntry { api_key: Some(key) });
                    }
                    Some(_) => {}
                }
            }
        }
        // Pass 2: stamped orphans fill endpoints still without a key.
        for (id, row) in &rows {
            let Some(key) = key_of(row) else { continue };
            if configured.iter().any(|m| &m.id == id) {
                continue;
            }
            match (&row.provider, &row.url) {
                (Some(p), Some(u)) if !p.is_empty() && !u.is_empty() => {
                    if out.endpoint_key(p, u).is_none() {
                        out.set_endpoint_key(p, u, Some(key));
                    }
                }
                _ => {
                    out.services.insert(id.clone(), KeyEntry { api_key: Some(key) });
                }
            }
        }
        out
    }

    /// Save to disk. Creates parent directories if needed.
    pub fn save(&self, file: &Path) -> anyhow::Result<()> {
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(file, json)?;
        Ok(())
    }

    /// The key stored for an endpoint.
    pub fn endpoint_key(&self, provider: &str, url: &str) -> Option<&str> {
        self.endpoints
            .iter()
            .find(|e| same_endpoint(&e.provider, &e.url, provider, url))
            .and_then(|e| e.api_key.as_deref())
            .filter(|k| !k.is_empty())
    }

    /// Set or clear an endpoint's key. Clearing removes the entry.
    pub fn set_endpoint_key(&mut self, provider: &str, url: &str, api_key: Option<String>) {
        self.endpoints
            .retain(|e| !same_endpoint(&e.provider, &e.url, provider, url));
        if let Some(key) = api_key.filter(|k| !k.is_empty()) {
            self.endpoints.push(EndpointKey { provider: provider.trim().to_string(), url: url.trim().to_string(), api_key: Some(key) });
            self.endpoints.sort_by(|a, b| (&a.provider, &a.url).cmp(&(&b.provider, &b.url)));
        }
    }

    /// A model's own key, when it overrides its endpoint's.
    pub fn model_override(&self, model_id: &str) -> Option<&str> {
        self.models
            .get(model_id)
            .and_then(|e| e.api_key.as_deref())
            .filter(|k| !k.is_empty())
    }

    /// Set or clear a model's override.
    pub fn set_model_override(&mut self, model_id: &str, api_key: Option<String>) {
        match api_key.filter(|k| !k.is_empty()) {
            Some(key) => {
                self.models.insert(model_id.to_string(), KeyEntry { api_key: Some(key) });
            }
            None => {
                self.models.remove(model_id);
            }
        }
    }

    /// A key that is not a model's (web search, a service).
    pub fn service_key(&self, name: &str) -> Option<&str> {
        self.services
            .get(name)
            .and_then(|e| e.api_key.as_deref())
            .filter(|k| !k.is_empty())
    }

    pub fn set_service_key(&mut self, name: &str, api_key: Option<String>) {
        match api_key.filter(|k| !k.is_empty()) {
            Some(key) => {
                self.services.insert(name.to_string(), KeyEntry { api_key: Some(key) });
            }
            None => {
                self.services.remove(name);
            }
        }
    }

    /// Drop overrides for models that are no longer configured — an
    /// override belongs to a model row; the endpoint key stays.
    pub fn prune_overrides(&mut self, configured: &[ModelConfig]) -> usize {
        let before = self.models.len();
        self.models
            .retain(|id, _| configured.iter().any(|m| &m.id == id));
        before - self.models.len()
    }

    /// Return a copy with all keys redacted (for API responses).
    pub fn redacted(&self) -> Self {
        let mask = |k: &Option<String>| k.as_ref().map(|_| "***".to_string());
        Self {
            version: self.version,
            endpoints: self
                .endpoints
                .iter()
                .map(|e| EndpointKey { provider: e.provider.clone(), url: e.url.clone(), api_key: mask(&e.api_key) })
                .collect(),
            models: self.models.iter().map(|(id, e)| (id.clone(), KeyEntry { api_key: mask(&e.api_key) })).collect(),
            services: self.services.iter().map(|(id, e)| (id.clone(), KeyEntry { api_key: mask(&e.api_key) })).collect(),
        }
    }
}

/// Default credentials file path: `~/.linggen/credentials.json`.
pub fn credentials_file() -> PathBuf {
    crate::paths::linggen_home().join("credentials.json")
}

fn env_key(name: &str) -> Option<String> {
    let var = format!("LINGGEN_API_KEY_{}", name.to_uppercase().replace(['-', '.', '/'], "_"));
    std::env::var(var).ok().filter(|k| !k.is_empty())
}

/// Resolve the effective API key for a model.
/// Priority: 1) TOML config api_key  2) the model's override  3) the
/// endpoint's key  4) env LINGGEN_API_KEY_{MODEL_ID}  5) env LINGGEN_API_KEY_{PROVIDER}
pub fn resolve_api_key(
    model: &ModelConfig,
    credentials: &Credentials,
) -> Option<String> {
    if let Some(key) = model.api_key.as_deref().filter(|k| !k.is_empty()) {
        return Some(key.to_string());
    }
    if let Some(key) = credentials.model_override(&model.id) {
        return Some(key.to_string());
    }
    if let Some(key) = credentials.endpoint_key(&model.provider, &model.url) {
        return Some(key.to_string());
    }
    env_key(&model.id).or_else(|| env_key(&model.provider))
}

/// The key the engine sends for a model. Kept as the one entry point every
/// caller uses; `_all` is no longer needed now that keys live per endpoint.
pub fn resolve_api_key_shared(
    model: &ModelConfig,
    _all: &[ModelConfig],
    credentials: &Credentials,
) -> Option<String> {
    resolve_api_key(model, credentials)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, provider: &str, url: &str) -> ModelConfig {
        toml::from_str(&format!(
            "id = \"{id}\"\nprovider = \"{provider}\"\nurl = \"{url}\"\nmodel = \"{id}\"\n"
        ))
        .unwrap()
    }

    const GEMINI: &str = "https://generativelanguage.googleapis.com/v1beta/openai";

    #[test]
    fn roundtrip_v2() {
        let tmp = std::env::temp_dir().join(format!("linggen_cred_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("credentials.json");

        let mut creds = Credentials::default();
        creds.set_endpoint_key("gemini", &format!("{GEMINI}/"), Some("AIza123".into()));
        creds.set_model_override("gemini-special", Some("AIzaOther".into()));
        creds.set_service_key("tavily", Some("tvly".into()));
        creds.save(&file).unwrap();

        let loaded = Credentials::load(&file);
        assert_eq!(loaded.version, 2);
        assert_eq!(loaded.endpoint_key("Gemini", GEMINI), Some("AIza123"));
        assert_eq!(loaded.model_override("gemini-special"), Some("AIzaOther"));
        assert_eq!(loaded.service_key("tavily"), Some("tvly"));
        assert_eq!(loaded.endpoint_key("groq", "https://api.groq.com/openai/v1"), None);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn every_model_on_the_endpoint_shares_the_key() {
        let mut creds = Credentials::default();
        creds.set_endpoint_key("gemini", GEMINI, Some("AIza".into()));
        let a = model("gemini-a", "gemini", GEMINI);
        let b = model("gemini-b", "Gemini", &format!("{GEMINI}/"));
        let other = model("groq-x", "groq", "https://api.groq.com/openai/v1");
        assert_eq!(resolve_api_key(&a, &creds), Some("AIza".into()));
        assert_eq!(resolve_api_key(&b, &creds), Some("AIza".into()));
        assert_eq!(resolve_api_key(&other, &creds), None);
        // An override wins over the endpoint; TOML over both.
        creds.set_model_override("gemini-b", Some("own".into()));
        assert_eq!(resolve_api_key(&b, &creds), Some("own".into()));
        let mut c = model("gemini-b", "gemini", GEMINI);
        c.api_key = Some("toml".into());
        assert_eq!(resolve_api_key(&c, &creds), Some("toml".into()));
    }

    #[test]
    fn v1_file_migrates_by_endpoint() {
        let tmp = std::env::temp_dir().join(format!("linggen_cred_v1_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("credentials.json");
        std::fs::write(&file, format!(r#"{{
  "stall-test": {{ "api_key": "x" }},
  "tavily": {{ "api_key": "tvly" }},
  "gemini-old-1": {{ "api_key": "AIza-old" }},
  "gemini-old-2": {{ "api_key": "AIza-old" }},
  "gemini-live": {{ "api_key": "AIza-live" }},
  "gemini-gone": {{ "api_key": "AQ-new", "provider": "gemini", "url": "{GEMINI}" }},
  "deepseek-v4-pro": {{ "api_key": "sk-ds" }},
  "gemini-two": {{ "api_key": "AIza-two" }}
}}"#)).unwrap();
        let configured = vec![
            model("gemini-live", "gemini", GEMINI),
            model("gemini-two", "gemini", GEMINI),
            model("deepseek-v4-pro", "deepseek", "https://api.deepseek.com/v1"),
        ];
        let creds = Credentials::load_for(&file, &configured);
        // The configured model's key is the endpoint's; the stamped orphan
        // with a different key did not win.
        assert_eq!(creds.endpoint_key("gemini", GEMINI), Some("AIza-live"));
        assert_eq!(creds.endpoint_key("deepseek", "https://api.deepseek.com/v1"), Some("sk-ds"));
        // A configured sibling with a different key keeps it as an override.
        assert_eq!(creds.model_override("gemini-two"), Some("AIza-two"));
        assert_eq!(creds.model_override("gemini-live"), None);
        // Unstamped rows naming no model are kept as services, not guessed.
        assert_eq!(creds.service_key("tavily"), Some("tvly"));
        assert_eq!(creds.service_key("gemini-old-1"), Some("AIza-old"));
        assert_eq!(creds.service_key("stall-test"), Some("x"));
        // Saved as v2, with the v1 copy kept.
        let on_disk: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(on_disk["version"], 2);
        assert!(file.with_extension("json.v1.bak").exists());
        // A stamped orphan alone still seeds its endpoint.
        let file2 = tmp.join("only-stamped.json");
        std::fs::write(&file2, format!(r#"{{ "gemini-gone": {{ "api_key": "AQ-new", "provider": "gemini", "url": "{GEMINI}" }} }}"#)).unwrap();
        let c2 = Credentials::load_for(&file2, &[model("gemini-live", "gemini", GEMINI)]);
        assert_eq!(c2.endpoint_key("gemini", GEMINI), Some("AQ-new"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn overrides_are_pruned_with_their_model() {
        let mut creds = Credentials::default();
        creds.set_model_override("gone", Some("k".into()));
        creds.set_model_override("kept", Some("k".into()));
        creds.set_endpoint_key("gemini", GEMINI, Some("AIza".into()));
        assert_eq!(creds.prune_overrides(&[model("kept", "gemini", GEMINI)]), 1);
        assert_eq!(creds.model_override("gone"), None);
        assert_eq!(creds.model_override("kept"), Some("k"));
        assert_eq!(creds.endpoint_key("gemini", GEMINI), Some("AIza"));
    }

    #[test]
    fn redacted_masks_every_key() {
        let mut creds = Credentials::default();
        creds.set_endpoint_key("gemini", GEMINI, Some("secret".into()));
        creds.set_model_override("m", Some("secret".into()));
        creds.set_service_key("tavily", Some("secret".into()));
        let r = creds.redacted();
        assert_eq!(r.endpoint_key("gemini", GEMINI), Some("***"));
        assert_eq!(r.model_override("m"), Some("***"));
        assert_eq!(r.service_key("tavily"), Some("***"));
        assert_eq!(r.endpoints[0].provider, "gemini");
    }

    #[test]
    fn clearing_removes() {
        let mut creds = Credentials::default();
        creds.set_endpoint_key("gemini", GEMINI, Some("k".into()));
        creds.set_endpoint_key("gemini", GEMINI, None);
        assert!(creds.endpoints.is_empty());
        creds.set_model_override("m", Some("k".into()));
        creds.set_model_override("m", Some(String::new()));
        assert!(creds.models.is_empty());
    }

    #[test]
    fn a_map_shaped_endpoints_section_still_reads_and_is_rewritten_as_a_list() {
        let tmp = std::env::temp_dir().join(format!("linggen_cred_map_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("credentials.json");
        std::fs::write(&file, format!(r#"{{ "version": 2, "endpoints": {{ "gemini|x": {{ "provider": "gemini", "url": "{GEMINI}", "api_key": "AIza" }} }}, "services": {{ "tavily": {{ "api_key": "t" }} }} }}"#)).unwrap();
        let creds = Credentials::load_for(&file, &[]);
        assert_eq!(creds.endpoint_key("gemini", GEMINI), Some("AIza"));
        let on_disk: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert!(on_disk["endpoints"].is_array(), "written back as a list");
        assert_eq!(on_disk["endpoints"][0]["provider"], "gemini");
        assert_eq!(on_disk["services"]["tavily"]["api_key"], "t");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn load_missing_file() {
        let creds = Credentials::load(Path::new("/nonexistent/credentials.json"));
        assert!(creds.endpoints.is_empty());
        assert_eq!(creds.version, 2);
    }
}

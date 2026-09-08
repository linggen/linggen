use crate::config::ModelConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::warn;

// ---------------------------------------------------------------------------
// Persisted format: ~/.linggen/credentials.json
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct CredentialEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// The endpoint the key was entered for, stamped on save. A key belongs
    /// to an endpoint, not to a model id: the stamp lets it keep serving the
    /// same provider + URL after the model row is renamed or replaced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct Credentials {
    /// Keyed by model ID (e.g. "gemini-flash", "groq-llama").
    #[serde(flatten)]
    pub entries: HashMap<String, CredentialEntry>,
}

impl Credentials {
    /// Load from `~/.linggen/credentials.json`. Returns empty if missing or invalid.
    pub fn load(file: &Path) -> Self {
        if !file.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(file) {
            Ok(content) => match serde_json::from_str::<Credentials>(&content) {
                Ok(creds) => creds,
                Err(e) => {
                    warn!("Failed to parse credentials.json: {}", e);
                    Self::default()
                }
            },
            Err(e) => {
                warn!("Failed to read credentials.json: {}", e);
                Self::default()
            }
        }
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

    /// Get the API key for a model ID.
    pub fn get_api_key(&self, model_id: &str) -> Option<&str> {
        self.entries
            .get(model_id)
            .and_then(|e| e.api_key.as_deref())
    }

    /// Set the API key for a model ID, keeping any endpoint stamp it has.
    pub fn set_api_key(&mut self, model_id: &str, api_key: Option<String>) {
        self.set_api_key_at(model_id, api_key, None);
    }

    /// Set the API key for a model ID and stamp the endpoint it belongs to.
    /// `None` for the endpoint leaves an existing stamp in place.
    pub fn set_api_key_at(
        &mut self,
        model_id: &str,
        api_key: Option<String>,
        endpoint: Option<(&str, &str)>,
    ) {
        if let Some(key) = api_key {
            let entry = self.entries.entry(model_id.to_string()).or_default();
            entry.api_key = Some(key);
            if let Some((provider, url)) = endpoint {
                entry.provider = Some(provider.to_string());
                entry.url = Some(url.to_string());
            }
        } else {
            // Remove the entry if key is None.
            self.entries.remove(model_id);
        }
    }

    /// The endpoint a stored key belongs to: its stamp, else the provider +
    /// URL of the configured model that still carries its id.
    fn endpoint_of<'a>(
        &'a self,
        id: &str,
        entry: &'a CredentialEntry,
        configured: &'a [ModelConfig],
    ) -> Option<(&'a str, &'a str)> {
        match (&entry.provider, &entry.url) {
            (Some(p), Some(u)) => Some((p.as_str(), u.as_str())),
            _ => configured
                .iter()
                .find(|m| m.id == id)
                .map(|m| (m.provider.as_str(), m.url.as_str())),
        }
    }

    /// A stored key for this endpoint, whichever model id it was entered
    /// under — a renamed or replaced model row must not orphan its key.
    /// Ids are visited in sorted order so the answer is stable.
    pub fn key_for_endpoint(
        &self,
        provider: &str,
        url: &str,
        configured: &[ModelConfig],
        except_id: &str,
    ) -> Option<&str> {
        let mut ids: Vec<&String> = self.entries.keys().collect();
        ids.sort();
        ids.into_iter().find_map(|id| {
            if id == except_id {
                return None;
            }
            let entry = self.entries.get(id)?;
            let key = entry.api_key.as_deref().filter(|k| !k.is_empty())?;
            let (p, u) = self.endpoint_of(id, entry, configured)?;
            same_endpoint(p, u, provider, url).then_some(key)
        })
    }

    /// Return a copy with all keys redacted (for API responses).
    pub fn redacted(&self) -> Self {
        let entries = self
            .entries
            .iter()
            .map(|(id, entry)| {
                let redacted = CredentialEntry {
                    api_key: entry.api_key.as_ref().map(|_| "***".to_string()),
                    provider: entry.provider.clone(),
                    url: entry.url.clone(),
                };
                (id.clone(), redacted)
            })
            .collect();
        Self { entries }
    }
}

/// Default credentials file path: `~/.linggen/credentials.json`.
pub fn credentials_file() -> PathBuf {
    crate::paths::linggen_home().join("credentials.json")
}

/// Resolve the effective API key for a model.
/// Priority: 1) TOML config api_key  2) credentials.json  3) env var LINGGEN_API_KEY_{ID}
pub fn resolve_api_key(
    model_id: &str,
    config_api_key: Option<&str>,
    credentials: &Credentials,
) -> Option<String> {
    // 1. TOML config (backward compatible)
    if let Some(key) = config_api_key {
        if !key.is_empty() {
            return Some(key.to_string());
        }
    }
    // 2. credentials.json
    if let Some(key) = credentials.get_api_key(model_id) {
        if !key.is_empty() {
            return Some(key.to_string());
        }
    }
    // 3. Environment variable: LINGGEN_API_KEY_GEMINI_FLASH (hyphens → underscores, uppercase)
    let env_name = format!(
        "LINGGEN_API_KEY_{}",
        model_id.to_uppercase().replace('-', "_")
    );
    if let Ok(key) = std::env::var(&env_name) {
        if !key.is_empty() {
            return Some(key);
        }
    }
    None
}

/// Resolve a model's key the way the engine sends it. A key belongs to an
/// endpoint, not to a model id: a model with no key of its own uses the key
/// of a sibling configured on the same provider and URL, else any stored key
/// stamped for that endpoint — so a second Gemini model works without
/// pasting the key twice, and replacing the model row does not orphan it.
pub fn resolve_api_key_shared(
    model: &ModelConfig,
    all: &[ModelConfig],
    credentials: &Credentials,
) -> Option<String> {
    resolve_api_key(&model.id, model.api_key.as_deref(), credentials)
        .or_else(|| {
            all.iter()
                .filter(|m| {
                    m.id != model.id
                        && same_endpoint(&m.provider, &m.url, &model.provider, &model.url)
                })
                .find_map(|m| resolve_api_key(&m.id, m.api_key.as_deref(), credentials))
        })
        .or_else(|| {
            credentials
                .key_for_endpoint(&model.provider, &model.url, all, &model.id)
                .map(str::to_string)
        })
}

fn same_endpoint(provider_a: &str, url_a: &str, provider_b: &str, url_b: &str) -> bool {
    provider_a.eq_ignore_ascii_case(provider_b)
        && url_a
            .trim_end_matches('/')
            .eq_ignore_ascii_case(url_b.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credentials_roundtrip() {
        let tmp = std::env::temp_dir().join("linggen_cred_test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("credentials.json");

        let mut creds = Credentials::default();
        creds.set_api_key("gemini-flash", Some("AIza123".to_string()));
        creds.set_api_key("groq-llama", Some("gsk_456".to_string()));
        creds.save(&file).unwrap();

        let loaded = Credentials::load(&file);
        assert_eq!(loaded.get_api_key("gemini-flash"), Some("AIza123"));
        assert_eq!(loaded.get_api_key("groq-llama"), Some("gsk_456"));
        assert_eq!(loaded.get_api_key("unknown"), None);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_credentials_redacted() {
        let mut creds = Credentials::default();
        creds.set_api_key("model-a", Some("secret".to_string()));
        let redacted = creds.redacted();
        assert_eq!(redacted.get_api_key("model-a"), Some("***"));
    }

    #[test]
    fn test_resolve_api_key_priority() {
        let mut creds = Credentials::default();
        creds.set_api_key("m1", Some("from_creds".to_string()));

        // TOML takes priority
        assert_eq!(
            resolve_api_key("m1", Some("from_toml"), &creds),
            Some("from_toml".to_string())
        );
        // Falls back to credentials
        assert_eq!(
            resolve_api_key("m1", None, &creds),
            Some("from_creds".to_string())
        );
        // No key at all
        assert_eq!(resolve_api_key("m2", None, &creds), None);
    }

    fn model(id: &str, provider: &str, url: &str) -> ModelConfig {
        toml::from_str(&format!(
            "id = \"{id}\"\nprovider = \"{provider}\"\nurl = \"{url}\"\nmodel = \"{id}\"\n"
        ))
        .unwrap()
    }

    #[test]
    fn test_shared_key_borrowed_from_same_endpoint() {
        let mut creds = Credentials::default();
        creds.set_api_key("gemini-old", Some("AIza".to_string()));
        let endpoint = "https://generativelanguage.googleapis.com/v1beta/openai";
        let old = model("gemini-old", "gemini", endpoint);
        // Same endpoint, trailing slash and case aside.
        let new = model("gemini-new", "Gemini", &format!("{endpoint}/"));
        let other = model("groq-x", "groq", "https://api.groq.com/openai/v1");
        let all = vec![old.clone(), new.clone(), other.clone()];

        assert_eq!(
            resolve_api_key_shared(&new, &all, &creds),
            Some("AIza".to_string())
        );
        assert_eq!(resolve_api_key_shared(&other, &all, &creds), None);

        // A key of its own still wins.
        creds.set_api_key("gemini-new", Some("own".to_string()));
        assert_eq!(
            resolve_api_key_shared(&new, &all, &creds),
            Some("own".to_string())
        );
    }

    #[test]
    fn test_stamped_key_survives_model_row_replacement() {
        let endpoint = "https://generativelanguage.googleapis.com/v1beta/openai";
        let mut creds = Credentials::default();
        // Entered for a model row that has since been deleted from config.
        creds.set_api_key_at(
            "gemini-old",
            Some("AIza".to_string()),
            Some(("gemini", endpoint)),
        );
        let new = model("gemini-new", "gemini", endpoint);
        let all = vec![new.clone()];
        assert_eq!(
            resolve_api_key_shared(&new, &all, &creds),
            Some("AIza".to_string())
        );

        // An unstamped orphan says nothing about its endpoint — no guess.
        let mut bare = Credentials::default();
        bare.set_api_key("gemini-old", Some("AIza".to_string()));
        assert_eq!(resolve_api_key_shared(&new, &all, &bare), None);

        // Stamps survive the redacted copy the settings page reads.
        let redacted = creds.redacted();
        assert_eq!(
            redacted.entries["gemini-old"].provider.as_deref(),
            Some("gemini")
        );
        assert_eq!(redacted.get_api_key("gemini-old"), Some("***"));
    }

    #[test]
    fn test_set_api_key_none_removes() {
        let mut creds = Credentials::default();
        creds.set_api_key("m1", Some("key".to_string()));
        assert!(creds.get_api_key("m1").is_some());
        creds.set_api_key("m1", None);
        assert!(creds.get_api_key("m1").is_none());
    }

    #[test]
    fn test_load_missing_file() {
        let creds = Credentials::load(Path::new("/nonexistent/credentials.json"));
        assert!(creds.entries.is_empty());
    }
}

//! The ChatGPT backend's own model list for the signed-in account
//! (`GET models?client_version=<v>`). The backend filters it by the version
//! presented, so it is read after the version. Only visible models are kept;
//! a failed or empty read keeps the last list — the catalog is never emptied.

use super::{Listed, Snapshot};
use async_trait::async_trait;

/// The signed-in account's backend. Tests pass their own.
#[async_trait]
pub trait Backend: Send + Sync {
    /// The backend's model list as it answers it.
    async fn models(&self, client_version: &str) -> anyhow::Result<serde_json::Value>;
}

/// The real backend, through the ChatGPT sign-in.
pub struct ChatgptBackend {
    client: crate::provider::openai::OpenAiClient,
}

impl ChatgptBackend {
    /// None when not signed in to ChatGPT.
    pub fn signed_in() -> Option<Self> {
        signed_in_client().map(|client| Self { client })
    }
}

/// A client on the ChatGPT sign-in, or None when not signed in.
pub fn signed_in_client() -> Option<crate::provider::openai::OpenAiClient> {
    use crate::provider::codex_auth;
    let tokens = codex_auth::CodexAuthTokens::load(&codex_auth::codex_auth_file());
    if !tokens.is_valid() {
        return None;
    }
    Some(crate::provider::openai::OpenAiClient::new_chatgpt_oauth(
        codex_auth::CHATGPT_API_BASE.to_string(),
        tokens.access_token.unwrap_or_default(),
        tokens.account_id,
    ))
}

#[async_trait]
impl Backend for ChatgptBackend {
    async fn models(&self, client_version: &str) -> anyhow::Result<serde_json::Value> {
        self.client
            .codex_metadata(&format!("models?client_version={client_version}"))
            .await
    }
}

/// The visible models in the backend's answer.
pub fn parse_list(json: &serde_json::Value) -> Vec<Listed> {
    let Some(models) = json.get("models").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    models.iter().filter_map(listed).collect()
}

/// One entry, when it is a visible model with a slug.
fn listed(m: &serde_json::Value) -> Option<Listed> {
    if m.get("visibility").and_then(|v| v.as_str()) == Some("hide") {
        return None;
    }
    let slug = m.get("slug")?.as_str()?.to_string();
    if slug.is_empty() {
        return None;
    }
    Some(Listed {
        slug,
        priority: m.get("priority").and_then(|v| v.as_i64()).unwrap_or(i64::MAX),
        context_window: m.get("context_window").and_then(|v| v.as_u64()).filter(|w| *w > 0),
        reasoning_levels: m.get("supported_reasoning_levels")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|level| level.get("effort").and_then(|v| v.as_str()))
            .filter(|effort| !effort.is_empty())
            .map(str::to_owned)
            .collect(),
        default_reasoning_level: m.get("default_reasoning_level").and_then(|v| v.as_str()).map(str::to_owned),
    })
}

/// Read the backend's list into [snap]. True when the list changed.
pub async fn read_list(snap: &mut Snapshot, backend: &dyn Backend, now: u64) -> bool {
    let json = match backend.models(&snap.client_version()).await {
        Ok(j) => j,
        Err(e) => {
            tracing::debug!("chatgpt catalog: model list unavailable: {e}");
            return false;
        }
    };
    let listed = parse_list(&json);
    if listed.is_empty() {
        tracing::debug!("chatgpt catalog: empty model list, keeping the last one");
        return false;
    }
    snap.listed_at = Some(now);
    if snap.listed == listed {
        return false;
    }
    snap.listed = listed;
    true
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The backend's answer on 2026-11-17 at client 0.169.x, trimmed to the
    /// keys read here plus a few beside them.
    pub(crate) fn measured_answer() -> serde_json::Value {
        let rows = [
            ("gpt-6.2-sol", "list", 1),
            ("gpt-6.2-astra", "list", 2),
            ("gpt-6.1-sol", "list", 3),
            ("gpt-6.1-terra", "list", 4),
            ("gpt-6-astra", "list", 5),
            ("gpt-6-sol", "list", 6),
            ("gpt-reserve", "hide", 7),
            ("gpt-6-luna", "list", 8),
            ("gpt-5.6-sol", "list", 9),
            ("gpt-5.6-terra", "list", 10),
            ("gpt-5.6-luna", "list", 11),
            ("codex-auto-review", "hide", 12),
        ];
        let models: Vec<serde_json::Value> = rows
            .iter()
            .map(|(slug, visibility, priority)| {
                serde_json::json!({
                    "slug": slug,
                    "display_name": slug,
                    "visibility": visibility,
                    "priority": priority,
                    "context_window": 272000,
                    "minimal_client_version": "0.150.0",
                    "supported_in_api": true,
                    "upgrade": null,
                })
            })
            .collect();
        serde_json::json!({ "models": models })
    }

    pub(crate) struct Answers(pub anyhow::Result<serde_json::Value>);

    #[async_trait]
    impl Backend for Answers {
        async fn models(&self, _v: &str) -> anyhow::Result<serde_json::Value> {
            match &self.0 {
                Ok(j) => Ok(j.clone()),
                Err(e) => Err(anyhow::anyhow!("{e}")),
            }
        }
    }

    #[test]
    fn only_visible_models_are_kept_in_the_backends_order() {
        let got = parse_list(&measured_answer());
        assert_eq!(got.len(), 10);
        assert!(!got.iter().any(|m| m.slug == "gpt-reserve" || m.slug == "codex-auto-review"));
        assert_eq!(got[0].slug, "gpt-6.2-sol");
        assert_eq!(got[0].priority, 1);
        assert_eq!(got[0].context_window, Some(272_000));
        assert!(got.iter().all(|m| m.reasoning_levels.is_empty() && m.default_reasoning_level.is_none()));
        assert_eq!(got, super::super::tests::measured_list());
    }

    #[tokio::test]
    async fn codex_reasoning_metadata_survives_a_snapshot_round_trip() {
        // The measured fixture is trimmed; add a synthetic Codex-shaped entry.
        let mut answer = measured_answer();
        answer["models"][0]["supported_reasoning_levels"] = serde_json::json!([
            {"effort": "low", "description": "Fast"},
            {"effort": "ultra"},
            {"effort": "future"},
            {"effort": 4},
        ]);
        answer["models"][0]["default_reasoning_level"] = "low".into();
        let mut snap = Snapshot::seed();
        assert!(read_list(&mut snap, &Answers(Ok(answer)), 50).await);
        assert_eq!(snap.reasoning_levels("gpt-6.2-sol"), ["low", "ultra", "future"]);
        assert_eq!(snap.listed[0].default_reasoning_level.as_deref(), Some("low"));
        assert!(snap.reasoning_levels("gpt-6.1-terra").is_empty());
        let restored: Snapshot = serde_json::from_value(serde_json::to_value(&snap).unwrap()).unwrap();
        assert_eq!(restored, snap);
    }

    #[tokio::test]
    async fn a_read_list_goes_into_the_snapshot() {
        let mut s = Snapshot::seed();
        assert!(read_list(&mut s, &Answers(Ok(measured_answer())), 50).await);
        assert_eq!(s.listed.len(), 10);
        assert_eq!(s.listed_at, Some(50));
        assert_eq!(s.context_window("gpt-6.1-terra"), Some(272_000));
    }

    #[tokio::test]
    async fn a_failed_or_empty_read_keeps_the_last_list() {
        let mut s = Snapshot::seed();
        read_list(&mut s, &Answers(Ok(measured_answer())), 50).await;
        assert!(!read_list(&mut s, &Answers(Err(anyhow::anyhow!("502"))), 60).await);
        assert!(!read_list(&mut s, &Answers(Ok(serde_json::json!({"models": []}))), 70).await);
        assert!(!read_list(&mut s, &Answers(Ok(serde_json::json!({"oops": 1}))), 80).await);
        assert_eq!(s.listed.len(), 10);
        assert_eq!(s.listed_at, Some(50));
    }
}

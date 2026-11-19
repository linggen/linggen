//! Can this model be used right now, and if not, why and what to do — the
//! one answer for a caller choosing a model (`agent_status` op=models) and
//! for a run that ended on a refusal.
//!
//! Read from what the engine already knows, without calling the model: the
//! credential pre-flight the stream runs ([`ModelManager::check_provider_auth`]),
//! the key a provider needs, and the health board ([`super::standing`]),
//! which keeps a usage limit's reset as the provider named it.

use super::{standing, ModelManager};
use crate::config::ModelConfig;
use crate::provider::error::ProviderErrorKind;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::SystemTime;

/// Why a model cannot be used now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotReady {
    /// A Linggen Cloud model, and no Linggen sign-in on this machine.
    NotSignedIn,
    /// A Linggen Cloud model whose sign-in the site has refused.
    SignInExpired,
    /// Another provider's sign-in (ChatGPT, Claude) is missing or expired.
    ProviderSignIn,
    /// A hosted provider with no API key.
    NoKey,
    /// The plan's usage window is used up until a time the provider named.
    UsageLimit,
    /// Too many requests just now; it lifts on its own shortly.
    RateLimited,
    /// Out of money or credit with the provider.
    QuotaUsedUp,
    /// The provider is failing (server errors, unreachable).
    ProviderFailing,
}

/// One model's readiness: `ready`, or the reason, the reset time when
/// there is one, and one plain line on what to do.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Readiness {
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<NotReady>,
    /// RFC 3339, when the provider named when it lifts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_in_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
}

impl Readiness {
    fn ready() -> Self {
        Self {
            ready: true,
            reason: None,
            resets_at: None,
            resets_in_secs: None,
            line: None,
        }
    }

    fn not(reason: NotReady, line: String) -> Self {
        Self {
            ready: false,
            reason: Some(reason),
            resets_at: None,
            resets_in_secs: None,
            line: Some(line),
        }
    }

    fn until(mut self, at: SystemTime) -> Self {
        let left = at.duration_since(SystemTime::now()).unwrap_or_default();
        self.resets_at = Some(rfc3339(at));
        self.resets_in_secs = Some(left.as_secs());
        self
    }
}

/// The line when no model at all can be used.
pub const NONE_READY: &str =
    "No model is ready; add one in Settings → Models or sign in to use Linggen Cloud.";

pub(crate) fn rfc3339(at: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(at).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

impl ModelManager {
    /// Whether model `id` can be used now; `None` for a model this manager
    /// does not have.
    pub fn readiness(&self, id: &str) -> Option<Readiness> {
        let cfg = self.list_models().into_iter().find(|m| m.id == id)?;
        if let Err(e) = Self::check_provider_auth(cfg) {
            return Some(signed_out(cfg, &e));
        }
        if needs_key(cfg) {
            return Some(Readiness::not(
                NotReady::NoKey,
                "Add this provider's API key in Settings → Models.".into(),
            ));
        }
        Some(match standing(id) {
            Some(s) => refusing(s.kind, s.until, s.reset_at),
            None => Readiness::ready(),
        })
    }

    /// Every model with its readiness, the default marked; when none can
    /// be used, one line on what to do. A ready local model means no
    /// sign-up is suggested.
    pub fn readiness_list(&self, default: Option<&str>) -> Value {
        let mut any_ready = false;
        let models: Vec<Value> = self
            .list_models()
            .iter()
            .map(|m| {
                let r = self.readiness(&m.id).unwrap_or_else(Readiness::ready);
                any_ready |= r.ready;
                let mut row = json!({
                    "id": m.id,
                    "provider": m.provider,
                    "default": Some(m.id.as_str()) == default,
                });
                if let Value::Object(fields) = serde_json::to_value(&r).unwrap_or_default() {
                    row.as_object_mut().unwrap().extend(fields);
                }
                row
            })
            .collect();
        let mut out = json!({ "models": models });
        if !any_ready {
            out["note"] = json!(NONE_READY);
        }
        out
    }
}

/// A sign-in the pre-flight refused. Linggen Cloud is told by the model's
/// auth mode, never by the message's words.
fn signed_out(cfg: &ModelConfig, err: &anyhow::Error) -> Readiness {
    if cfg.auth_mode.as_deref() == Some("linggen_account") {
        return match cloud_sign_in() {
            NotReady::SignInExpired => Readiness::not(
                NotReady::SignInExpired,
                "Linggen sign-in expired: run `ling account login` again, or pick another model."
                    .into(),
            ),
            reason => Readiness::not(
                reason,
                "Needs a Linggen account: sign in with `ling account login`, or pick another model."
                    .into(),
            ),
        };
    }
    let said = err.to_string();
    let line = said
        .strip_prefix("AUTH_REQUIRED: ")
        .unwrap_or(&said)
        .to_string();
    Readiness::not(NotReady::ProviderSignIn, line)
}

/// The one rule for a usage limit: a rate limit or a spent quota whose
/// provider named when it lifts. Readiness and a run's typed reason
/// (`cannot_run`) both ask this.
pub(crate) fn usage_limit_until(
    kind: ProviderErrorKind,
    reset_at: Option<SystemTime>,
) -> Option<SystemTime> {
    let limited = matches!(
        kind,
        ProviderErrorKind::RateLimit | ProviderErrorKind::QuotaExhausted
    );
    reset_at.filter(|_| limited)
}

/// What to do about a usage limit, in one line.
pub(crate) fn usage_limit_line(model: Option<&str>, resets_at: &str) -> String {
    let on = model.map(|m| format!(" on {m}")).unwrap_or_default();
    format!(
        "Usage limit{on} until {resets_at}; pick a ready model (agent_status op=models) or come back after it — never retry before."
    )
}

/// Whether this machine has no Linggen sign-in, or one the site refused:
/// the one reading for a Linggen Cloud model's refusal.
pub(crate) fn cloud_sign_in() -> NotReady {
    match crate::account::resolve_token() {
        None => NotReady::NotSignedIn,
        Some(_) => NotReady::SignInExpired,
    }
}

/// What the health board's standing refusal means for a caller.
pub(crate) fn refusing(
    kind: ProviderErrorKind,
    until: SystemTime,
    reset_at: Option<SystemTime>,
) -> Readiness {
    if let Some(at) = usage_limit_until(kind, reset_at) {
        return Readiness::not(NotReady::UsageLimit, usage_limit_line(None, &rfc3339(at)))
            .until(at);
    }
    match (kind, reset_at) {
        (ProviderErrorKind::RateLimit, None) => Readiness::not(
            NotReady::RateLimited,
            "Rate-limited just now; pick another model or try again shortly.".into(),
        )
        .until(until),
        (ProviderErrorKind::QuotaExhausted, None) => Readiness::not(
            NotReady::QuotaUsedUp,
            "Out of credit with this provider; top it up or pick another model.".into(),
        ),
        _ => Readiness::not(
            NotReady::ProviderFailing,
            "The provider is failing right now; pick another model or try again shortly.".into(),
        )
        .until(until),
    }
}

/// A hosted provider reached with no key. A local or LAN server (Ollama,
/// an OpenAI-compatible server on this network) needs none, nor does a
/// model signed in another way or a room's proxy. `auth_mode = "api_key"`
/// names the key itself, so it is no sign-in.
fn needs_key(cfg: &ModelConfig) -> bool {
    let has_key = cfg.api_key.as_deref().is_some_and(|k| !k.trim().is_empty());
    // The model manager treats a `chatgpt` provider as signed in with
    // ChatGPT whatever its auth mode says; so does this.
    let signs_in = cfg.auth_mode.as_deref().is_some_and(|m| m != "api_key")
        || cfg.provider == "chatgpt";
    let keyless = matches!(cfg.provider.as_str(), "ollama" | "proxy");
    !(has_key || signs_in || keyless || on_this_network(&cfg.url))
}

/// Is `url`'s host this machine or a private network address?
fn on_this_network(url: &str) -> bool {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let authority = rest.split('/').next().unwrap_or("");
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => authority
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(authority),
    };
    if host.is_empty() || host == "localhost" || host.ends_with(".local") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Ok(std::net::IpAddr::V6(ip)) => ip.is_loopback(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::models::note_unavailable;
    use std::time::Duration;

    fn model(
        id: &str,
        provider: &str,
        url: &str,
        key: Option<&str>,
        auth: Option<&str>,
    ) -> ModelConfig {
        let mut cfg: ModelConfig = serde_json::from_value(json!({
            "id": id, "provider": provider, "url": url, "model": id,
        }))
        .unwrap();
        cfg.api_key = key.map(str::to_string);
        cfg.auth_mode = auth.map(str::to_string);
        cfg
    }

    /// A local model is ready with no key and no account; a hosted one
    /// without a key is not, and says what to do.
    #[test]
    fn local_models_need_nothing_and_hosted_ones_need_a_key() {
        for (url, provider) in [
            ("http://127.0.0.1:11434", "ollama"),
            ("http://localhost:1234/v1", "openai"),
            ("http://192.168.1.20:8000/v1", "openai"),
            ("http://[::1]:8080/v1", "openai"),
        ] {
            assert!(!needs_key(&model("m", provider, url, None, None)), "{url}");
        }
        assert!(needs_key(&model(
            "m",
            "openai",
            "https://api.openai.com/v1",
            None,
            None
        )));
        assert!(needs_key(&model(
            "m",
            "anthropic",
            "https://api.anthropic.com",
            Some(" "),
            None
        )));
        assert!(!needs_key(&model(
            "m",
            "anthropic",
            "https://api.anthropic.com",
            Some("k"),
            None
        )));
        assert!(!needs_key(&model(
            "m",
            "openai",
            "https://x",
            None,
            Some("chatgpt_oauth")
        )));
        // An auth mode that names the key is no sign-in: with no key, the
        // model is not ready.
        assert!(needs_key(&model(
            "m",
            "openai",
            "https://api.openai.com/v1",
            None,
            Some("api_key")
        )));
        assert!(!needs_key(&model(
            "m",
            "chatgpt",
            "https://chatgpt.com/backend-api",
            None,
            None
        )));
    }

    /// A usage limit is a fact with its reset; a blind rate limit and a
    /// failing provider say when they are tried again.
    #[test]
    fn a_usage_limit_names_its_reset_time() {
        let now = SystemTime::now();
        let reset = now + Duration::from_secs(8973);
        let r = refusing(
            ProviderErrorKind::RateLimit,
            now + Duration::from_secs(3600),
            Some(reset),
        );
        assert!(!r.ready);
        assert_eq!(r.reason, Some(NotReady::UsageLimit));
        assert_eq!(r.resets_at.as_deref(), Some(rfc3339(reset).as_str()));
        assert!((8900..=8973).contains(&r.resets_in_secs.unwrap()));
        let line = r.line.unwrap();
        assert!(
            line.starts_with("Usage limit until ") && line.contains("come back after it"),
            "{line}"
        );

        let blind = refusing(
            ProviderErrorKind::RateLimit,
            now + Duration::from_secs(60),
            None,
        );
        assert_eq!(blind.reason, Some(NotReady::RateLimited));
        let down = refusing(
            ProviderErrorKind::Server5xx,
            now + Duration::from_secs(60),
            None,
        );
        assert_eq!(down.reason, Some(NotReady::ProviderFailing));
        let broke = refusing(ProviderErrorKind::QuotaExhausted, now, None);
        assert_eq!(broke.reason, Some(NotReady::QuotaUsedUp));
    }

    /// The list: every model with its readiness. With one local model
    /// ready, nothing pushes a sign-up; with none ready, one line says what
    /// to do.
    #[test]
    fn the_list_marks_each_model_and_says_what_to_do_only_when_none_is_ready() {
        let _board = crate::provider::models::board_test_guard();
        let local = model(
            "ready-local",
            "ollama",
            "http://127.0.0.1:11434",
            None,
            None,
        );
        let hosted = model(
            "readiness-hosted",
            "acme",
            "https://api.acme.example/v1",
            None,
            None,
        );
        let limited = model(
            "readiness-limited",
            "acme",
            "https://api.acme.example/v1",
            Some("k"),
            None,
        );
        let body = r#"{"error":{"type":"usage_limit_reached","resets_in_seconds":7200}}"#;
        note_unavailable(
            "readiness-limited",
            &anyhow::anyhow!("openai error (429 Too Many Requests): {body}"),
        );
        let models = ModelManager::new_with_credentials(
            vec![local.clone(), hosted.clone(), limited.clone()],
            &Default::default(),
        );
        let list = models.readiness_list(Some("ready-local"));
        let row = |id: &str| {
            list["models"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["id"] == id)
                .cloned()
                .unwrap()
        };
        assert_eq!(row("ready-local")["ready"], true);
        assert_eq!(row("ready-local")["default"], true);
        assert!(row("ready-local").get("line").is_none());
        assert_eq!(row("readiness-hosted")["reason"], "no_key");
        assert_eq!(row("readiness-limited")["reason"], "usage_limit");
        assert!(row("readiness-limited")["resets_at"].is_string());
        assert!(
            list.get("note").is_none(),
            "a ready model means no sign-up push"
        );

        let none = ModelManager::new_with_credentials(vec![hosted], &Default::default());
        assert_eq!(none.readiness_list(None)["note"], NONE_READY);
        crate::provider::models::note_healthy("readiness-limited");
    }
}

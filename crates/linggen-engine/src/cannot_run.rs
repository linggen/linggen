//! Why something cannot run, typed once: no model, a Linggen Cloud
//! sign-in, a usage limit with its reset, another provider's refusal, the
//! browser extension not connected, the memory store unavailable.
//!
//! The same reason answers wherever the failure surfaces — refused at once,
//! the end of a watched run, a status read, a dream run's `last_run_error`
//! — so a caller acts on a field, not on words it has to parse. A refusal
//! that is no reason to give up (the user declined a browser prompt, a
//! request the store refused as bad) is not one of these.

use crate::provider::error::{kind_of, kind_of_message, named_reset, retry_after_of, ProviderErrorKind};
use crate::provider::models::rfc3339;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::{Duration, SystemTime};

/// What the engine says when no model exists to run a turn.
pub const NO_MODELS_CONFIGURED: &str = "No models configured";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum CannotRun {
    /// No generative model is configured.
    NoModel,
    /// A Linggen Cloud model and no (or a refused) Linggen sign-in.
    CloudSignIn {
        model: Option<String>,
        expired: bool,
    },
    /// The model's usage window is used up until the time its provider
    /// named.
    UsageLimit {
        model: Option<String>,
        resets_at: String,
        resets_in_secs: u64,
    },
    /// Another provider refusal: its kind, as the provider error knows it.
    Provider {
        model: Option<String>,
        kind: ProviderErrorKind,
    },
    /// The Linggen Browser extension is not connected.
    ExtensionNotConnected,
    /// The memory store did not answer, or is not open.
    StoreUnavailable { detail: String },
}

impl CannotRun {
    /// A model call's failure, read from the error itself: its provider
    /// kind and named reset. A Linggen Cloud sign-in is told by the model's
    /// auth mode.
    pub fn of_model_error(
        err: &anyhow::Error,
        model: Option<&str>,
        auth_mode: Option<&str>,
    ) -> Option<Self> {
        let text = format!("{err:#}");
        if text.contains(NO_MODELS_CONFIGURED) {
            return Some(Self::NoModel);
        }
        let kind = kind_of(err);
        let cloud = auth_mode == Some("linggen_account");
        provider(kind, retry_after_of(err), model, cloud)
    }

    /// A failure that survived only as text — a finished run's error, a
    /// dream run's `last_run_error`. Linggen Cloud is recognised by the
    /// sign-in line the engine itself writes for it.
    pub fn of_message(msg: &str, model: Option<&str>) -> Option<Self> {
        if msg.contains(NO_MODELS_CONFIGURED) {
            return Some(Self::NoModel);
        }
        if let Some(store) = Self::of_memory_error(msg) {
            return Some(store);
        }
        if msg.contains("browser not connected") {
            return Some(Self::ExtensionNotConnected);
        }
        let kind = kind_of_message(msg);
        let cloud = msg.contains("AUTH_REQUIRED") && msg.contains("linggen.dev");
        provider(kind, named_reset(msg), model, cloud)
    }

    /// A browser bridge refusal: only a missing connection is a reason
    /// the call cannot run (a declined prompt is the user's answer).
    pub fn of_bridge(code: &str) -> Option<Self> {
        matches!(code, "no_bridge" | "module_unavailable").then_some(Self::ExtensionNotConnected)
    }

    /// Memory's answer: a store that is not open answers 503
    /// `MEMORY_UNAVAILABLE` with its reason.
    pub fn of_memory_answer(status: u16, envelope: &Value) -> Option<Self> {
        let unavailable = envelope["code"] == "MEMORY_UNAVAILABLE";
        if status != 503 || !unavailable {
            return None;
        }
        let detail = envelope["error"].as_str().unwrap_or_default().to_string();
        Some(Self::StoreUnavailable { detail })
    }

    /// A memory call that got no answer at all: none in this process, or
    /// none in time.
    pub fn of_memory_error(msg: &str) -> Option<Self> {
        let unreachable = msg.contains("memory is not reachable")
            || msg.contains("memory is not hosted by this process")
            || msg.contains("MEMORY_UNAVAILABLE");
        unreachable.then(|| Self::StoreUnavailable {
            detail: msg.to_string(),
        })
    }

    /// Hit a usage limit: never retry before its reset.
    pub fn is_usage_limit(&self) -> bool {
        matches!(self, Self::UsageLimit { .. })
    }

    /// One plain line on what to do.
    pub fn line(&self) -> String {
        let named = |model: &Option<String>| model.clone().unwrap_or_else(|| "the model".into());
        match self {
            Self::NoModel => "To run a Linggen agent, add a model in Settings → Models or sign in to use Linggen Cloud.".into(),
            Self::CloudSignIn { .. } => "This model needs a Linggen account; create one or sign in at linggen.dev with `ling account login`, or choose another model in Settings → Models.".into(),
            Self::UsageLimit { model, resets_at, .. } => format!(
                "Usage limit on {} until {resets_at}; pick a ready model (agent_status op=models) or come back after it — never retry before.",
                named(model)
            ),
            Self::Provider { model, kind: ProviderErrorKind::Auth } => format!(
                "{}'s sign-in or key was refused; fix it in Settings → Models or choose another model.",
                named(model)
            ),
            Self::Provider { model, kind } => format!(
                "{} failed ({}); pick another model or try again.",
                named(model),
                kind.label()
            ),
            Self::ExtensionNotConnected => "Install or enable the Linggen Browser extension in Chrome, then retry.".into(),
            Self::StoreUnavailable { .. } => "Memory is unavailable; start Linggen or check `ling status`.".into(),
        }
    }

    /// [`line`](Self::line) for the dream run, which a host can always do
    /// with its own model instead.
    pub fn dream_line(&self) -> String {
        match self {
            Self::NoModel => "To run the dream in Linggen, configure a model in Settings → Models, or use /linggen:dream with your host's model.".into(),
            other => other.line(),
        }
    }

    /// The reason as a field of its own: `{reason, …, line}`.
    pub fn fact(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or_else(|_| json!({}));
        v["line"] = json!(self.line());
        v
    }
}

fn provider(
    kind: ProviderErrorKind,
    reset: Option<Duration>,
    model: Option<&str>,
    cloud: bool,
) -> Option<CannotRun> {
    let model = model.map(str::to_string);
    if kind == ProviderErrorKind::Auth && cloud {
        let expired = crate::account::resolve_token().is_some();
        return Some(CannotRun::CloudSignIn { model, expired });
    }
    let limited = matches!(kind, ProviderErrorKind::RateLimit | ProviderErrorKind::QuotaExhausted);
    if let (true, Some(left)) = (limited, reset) {
        return Some(CannotRun::UsageLimit {
            model,
            resets_at: rfc3339(SystemTime::now() + left),
            resets_in_secs: left.as_secs(),
        });
    }
    (kind != ProviderErrorKind::Other).then_some(CannotRun::Provider { model, kind })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::error::ProviderError;

    fn http(status: u16, body: &str) -> anyhow::Error {
        let status = reqwest::StatusCode::from_u16(status).unwrap();
        ProviderError::http(status, body, format!("openai error ({status}): {body}")).into()
    }

    const USAGE: &str = r#"{"error":{"type":"usage_limit_reached","message":"The usage limit has been reached","resets_at":1788973633,"resets_in_seconds":8973}}"#;

    /// A usage limit is the reason, the model and the reset — read from the
    /// provider error, not from its words.
    #[test]
    fn a_usage_limit_carries_its_model_and_reset() {
        let why = CannotRun::of_model_error(&http(429, USAGE), Some("gpt-x"), None).unwrap();
        let CannotRun::UsageLimit { model, resets_in_secs, .. } = &why else {
            panic!("{why:?}");
        };
        assert_eq!(model.as_deref(), Some("gpt-x"));
        assert!((8970..=8973).contains(resets_in_secs), "{resets_in_secs}");
        assert!(why.is_usage_limit());
        let fact = why.fact();
        assert_eq!(fact["reason"], "usage_limit");
        assert!(fact["resets_at"].is_string());
        assert!(fact["line"].as_str().unwrap().contains("never retry before"));

        // The same run's failure, as the text a finished frame carries.
        let text = format!("MODEL_FAILED: model 'gpt-x' failed (rate limit): openai error (429 Too Many Requests): {USAGE}");
        let again = CannotRun::of_message(&text, Some("gpt-x")).unwrap();
        assert!(again.is_usage_limit(), "{again:?}");

        // A rate limit with no named reset is a provider refusal, not a limit.
        let blind = CannotRun::of_model_error(&http(429, "{}"), Some("m"), None).unwrap();
        assert_eq!(
            blind,
            CannotRun::Provider { model: Some("m".into()), kind: ProviderErrorKind::RateLimit }
        );
    }

    /// Linggen Cloud's sign-in is its own reason, told by the auth mode; a
    /// ChatGPT or Claude sign-in is that provider's refusal, never a
    /// Linggen account prompt.
    #[test]
    fn only_a_linggen_cloud_model_asks_for_a_linggen_account() {
        let refused = anyhow::anyhow!(
            "AUTH_REQUIRED: Not signed in to linggen.dev for model 'cloud'. Run `ling account login` or sign in from the app."
        );
        let cloud = CannotRun::of_model_error(&refused, Some("cloud"), Some("linggen_account")).unwrap();
        assert!(matches!(cloud, CannotRun::CloudSignIn { .. }), "{cloud:?}");
        assert!(cloud.line().contains("Linggen account"));
        let by_text = CannotRun::of_message(&refused.to_string(), Some("cloud")).unwrap();
        assert!(matches!(by_text, CannotRun::CloudSignIn { .. }), "{by_text:?}");

        let chatgpt = anyhow::anyhow!("AUTH_REQUIRED: ChatGPT not signed in for model 'g'. Open Settings → Models → Sign in with ChatGPT.");
        let other = CannotRun::of_model_error(&chatgpt, Some("g"), Some("chatgpt_oauth")).unwrap();
        assert_eq!(other, CannotRun::Provider { model: Some("g".into()), kind: ProviderErrorKind::Auth });
        assert!(!other.line().contains("Linggen account"), "{}", other.line());
        let by_text = CannotRun::of_message(&chatgpt.to_string(), Some("g")).unwrap();
        assert!(!matches!(by_text, CannotRun::CloudSignIn { .. }), "{by_text:?}");
    }

    /// No model at all — from the engine's own error or a dream run's
    /// recorded failure — says to add one, and for the dream that the host
    /// can run it with its own model.
    #[test]
    fn no_model_is_named_wherever_it_surfaces() {
        let err = anyhow::anyhow!(NO_MODELS_CONFIGURED);
        assert_eq!(CannotRun::of_model_error(&err, None, None), Some(CannotRun::NoModel));
        let recorded = format!("failed to create agent 'ling': {NO_MODELS_CONFIGURED}");
        let why = CannotRun::of_message(&recorded, None).unwrap();
        assert_eq!(why, CannotRun::NoModel);
        assert!(why.dream_line().contains("/linggen:dream"));
        assert!(why.line().contains("Settings → Models"));
    }

    /// The extension and the store: a missing connection is a reason; the
    /// user's declined prompt and the store's ordinary refusals are not.
    #[test]
    fn the_extension_and_the_store_have_their_own_reasons() {
        assert_eq!(CannotRun::of_bridge("no_bridge"), Some(CannotRun::ExtensionNotConnected));
        assert_eq!(CannotRun::of_bridge("module_unavailable"), Some(CannotRun::ExtensionNotConnected));
        assert_eq!(CannotRun::of_bridge("not_permitted"), None);

        let closed = json!({"ok": false, "error": "store locked", "code": "MEMORY_UNAVAILABLE"});
        assert_eq!(
            CannotRun::of_memory_answer(503, &closed),
            Some(CannotRun::StoreUnavailable { detail: "store locked".into() })
        );
        let bad = json!({"ok": false, "error": "content must not be empty", "code": "BAD_REQUEST"});
        assert_eq!(CannotRun::of_memory_answer(400, &bad), None);
        let silent = CannotRun::of_memory_error("memory is not reachable — check `ling status`").unwrap();
        assert!(silent.line().contains("ling status"));
        assert_eq!(CannotRun::of_message("invalid JSON in response", None), None);
    }
}

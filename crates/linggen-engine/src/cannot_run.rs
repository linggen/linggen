//! Why something cannot run, typed once: no model, a Linggen Cloud
//! sign-in, a usage limit with its reset, another provider's refusal, the
//! browser extension not connected, the memory store unavailable.
//!
//! The same reason answers wherever the failure surfaces — refused at once,
//! the end of a watched run, a status read, a dream run's `last_run_error`
//! — so a caller acts on a field, not on words it has to parse. A refusal
//! that is no reason to give up (the user declined a browser prompt, a
//! request the store refused as bad) is not one of these.

use crate::provider::error::{kind_of, retry_after_of, ProviderErrorKind};
use crate::provider::models::{
    cloud_sign_in, rfc3339, usage_limit_line, usage_limit_until, NotReady,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::{Duration, SystemTime};

/// What the engine says when no model exists to run a turn.
pub const NO_MODELS_CONFIGURED: &str = "No models configured";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// The turn took every step its session allows and was not finished.
    /// Not a refusal: the request was fine, the job long — the same session
    /// continues it.
    StepLimit { steps: usize },
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

    /// An agent that could not be made: no model configured, or a
    /// provider's refusal in the error's chain. Anything else (a missing
    /// agent file, a store error) is no reason a caller acts on — an
    /// `io::Error` is never read as a provider's network failure.
    pub fn of_creation_error(err: &anyhow::Error) -> Option<Self> {
        if format!("{err:#}").contains(NO_MODELS_CONFIGURED) {
            return Some(Self::NoModel);
        }
        let from_provider = err.chain().any(|c| {
            c.downcast_ref::<crate::provider::error::ProviderError>()
                .is_some()
        });
        from_provider
            .then(|| Self::of_model_error(err, None, None))
            .flatten()
    }

    /// A browser bridge refusal: only a missing connection is a reason
    /// the call cannot run (a declined prompt is the user's answer). The
    /// one place this is decided; the door's browser branches carry it.
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

    /// A memory call that got no answer because there is no store: none in
    /// this process. One that got none in time is no such reason — its verb
    /// may still finish, so a write may have landed
    /// (`memory_host::NoAnswerInTime`, said as memory's own runner says it).
    pub fn of_memory_error(msg: &str) -> Option<Self> {
        let unreachable = msg.contains("memory is not hosted by this process")
            || msg.contains("MEMORY_UNAVAILABLE");
        unreachable.then(|| Self::StoreUnavailable {
            detail: msg.to_string(),
        })
    }

    /// The run ended at its step limit, not on a failure: its status is
    /// still `completed`, and its work is held for the session's next turn.
    pub fn is_step_limit(&self) -> bool {
        matches!(self, Self::StepLimit { .. })
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
            Self::UsageLimit { model, resets_at, .. } => usage_limit_line(model.as_deref(), resets_at),
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
            Self::StepLimit { steps } => format!(
                "This run reached its limit of {steps} tool-call steps; it can be continued in the same session."
            ),
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

    /// The reason as a field of its own: `{reason, …, line}`. A usage
    /// limit's time left is counted when the fact is read.
    pub fn fact(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or_else(|_| json!({}));
        if let Self::UsageLimit { resets_at, .. } = self {
            v["resets_in_secs"] = json!(secs_until(resets_at));
        }
        v["line"] = json!(self.line());
        v
    }

    /// [`fact`](Self::fact) for a run that ended on it, and whether its
    /// session's engine holds the run's work for `agent_run(session_id)` to
    /// continue from (it does until the engine is rebuilt: a restart, a
    /// settings change).
    pub fn run_fact(&self, work_kept: bool) -> Value {
        let mut v = self.fact();
        v["work_kept"] = json!(work_kept);
        v
    }

    /// [`fact`](Self::fact) for a dream run, its line
    /// [`dream_line`](Self::dream_line).
    pub fn dream_fact(&self) -> Value {
        let mut v = self.fact();
        v["line"] = json!(self.dream_line());
        v
    }
}

/// Whole seconds from now until an RFC 3339 time; 0 once it is past.
fn secs_until(at: &str) -> u64 {
    chrono::DateTime::parse_from_rfc3339(at)
        .map(|t| (t.timestamp() - chrono::Utc::now().timestamp()).max(0) as u64)
        .unwrap_or(0)
}

fn provider(
    kind: ProviderErrorKind,
    reset: Option<Duration>,
    model: Option<&str>,
    cloud: bool,
) -> Option<CannotRun> {
    let model = model.map(str::to_string);
    if kind == ProviderErrorKind::Auth && cloud {
        let expired = cloud_sign_in() == NotReady::SignInExpired;
        return Some(CannotRun::CloudSignIn { model, expired });
    }
    let now = SystemTime::now();
    if let Some(at) = usage_limit_until(kind, reset.map(|left| now + left)) {
        return Some(CannotRun::UsageLimit {
            model,
            resets_at: rfc3339(at),
            resets_in_secs: at.duration_since(now).unwrap_or_default().as_secs(),
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
        let CannotRun::UsageLimit {
            model,
            resets_in_secs,
            ..
        } = &why
        else {
            panic!("{why:?}");
        };
        assert_eq!(model.as_deref(), Some("gpt-x"));
        assert!((8970..=8973).contains(resets_in_secs), "{resets_in_secs}");
        assert!(why.is_usage_limit());
        let fact = why.fact();
        assert_eq!(fact["reason"], "usage_limit");
        assert!(fact["resets_at"].is_string());
        assert!(fact["line"]
            .as_str()
            .unwrap()
            .contains("never retry before"));

        // A rate limit with no named reset is a provider refusal, not a limit.
        let blind = CannotRun::of_model_error(&http(429, "{}"), Some("m"), None).unwrap();
        assert_eq!(
            blind,
            CannotRun::Provider {
                model: Some("m".into()),
                kind: ProviderErrorKind::RateLimit
            }
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
        let cloud =
            CannotRun::of_model_error(&refused, Some("cloud"), Some("linggen_account")).unwrap();
        assert!(matches!(cloud, CannotRun::CloudSignIn { .. }), "{cloud:?}");
        assert!(cloud.line().contains("Linggen account"));

        let chatgpt = anyhow::anyhow!("AUTH_REQUIRED: ChatGPT not signed in for model 'g'. Open Settings → Models → Sign in with ChatGPT.");
        let other = CannotRun::of_model_error(&chatgpt, Some("g"), Some("chatgpt_oauth")).unwrap();
        assert_eq!(
            other,
            CannotRun::Provider {
                model: Some("g".into()),
                kind: ProviderErrorKind::Auth
            }
        );
        assert!(
            !other.line().contains("Linggen account"),
            "{}",
            other.line()
        );
    }

    /// A session that may not switch models ends on its model's usage
    /// limit with the reset its provider named, kept through the error
    /// that says no other model was tried.
    #[test]
    fn a_usage_limit_keeps_its_reset_when_no_other_model_is_tried() {
        let ended = crate::provider::error::no_fallback_error("gpt-x", &http(429, USAGE));
        let why = CannotRun::of_model_error(&ended, Some("gpt-x"), None).unwrap();
        let CannotRun::UsageLimit {
            model, resets_at, ..
        } = &why
        else {
            panic!("{why:?}");
        };
        assert_eq!(model.as_deref(), Some("gpt-x"));
        let left = why.fact()["resets_in_secs"].as_u64().unwrap();
        assert!((8960..=8973).contains(&left), "{left} until {resets_at}");
        assert_eq!(why.run_fact(true)["work_kept"], true);
    }

    /// A model stream that timed out is a provider's network failure.
    #[test]
    fn a_model_stream_that_timed_out_is_a_network_failure() {
        let stream = anyhow::Error::from(ProviderError::new(
            ProviderErrorKind::Network,
            "stream timed out".into(),
        ));
        assert_eq!(
            CannotRun::of_model_error(&stream, Some("m"), None),
            Some(CannotRun::Provider {
                model: Some("m".into()),
                kind: ProviderErrorKind::Network
            })
        );
    }

    /// An agent that could not be made is a reason only for no model or a
    /// provider's refusal: a file that is not there is no network failure.
    #[test]
    fn a_creation_error_is_a_reason_only_when_it_is_one() {
        let none = anyhow::anyhow!(NO_MODELS_CONFIGURED).context("failed to create agent 'ling'");
        assert_eq!(
            CannotRun::of_creation_error(&none),
            Some(CannotRun::NoModel)
        );
        let missing = anyhow::Error::from(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "agents/ling.md",
        ));
        assert_eq!(
            CannotRun::of_model_error(&missing, None, None).map(|c| c.is_usage_limit()),
            Some(false),
            "read as a provider's"
        );
        assert_eq!(CannotRun::of_creation_error(&missing), None);
        let refused = http(401, "{}").context("failed to create agent 'ling'");
        assert!(matches!(
            CannotRun::of_creation_error(&refused),
            Some(CannotRun::Provider { .. })
        ));
    }

    /// No model at all says to add one, and for the dream that the host can
    /// run it with its own model.
    #[test]
    fn no_model_is_named_wherever_it_surfaces() {
        let err = anyhow::anyhow!(NO_MODELS_CONFIGURED);
        assert_eq!(
            CannotRun::of_model_error(&err, None, None),
            Some(CannotRun::NoModel)
        );
        let why = CannotRun::NoModel;
        assert!(why.dream_line().contains("/linggen:dream"));
        assert!(why.line().contains("Settings → Models"));
    }

    /// The extension and the store: a missing connection is a reason; the
    /// user's declined prompt and the store's ordinary refusals are not.
    #[test]
    fn the_extension_and_the_store_have_their_own_reasons() {
        assert_eq!(
            CannotRun::of_bridge("no_bridge"),
            Some(CannotRun::ExtensionNotConnected)
        );
        assert_eq!(
            CannotRun::of_bridge("module_unavailable"),
            Some(CannotRun::ExtensionNotConnected)
        );
        assert_eq!(CannotRun::of_bridge("not_permitted"), None);

        let closed = json!({"ok": false, "error": "store locked", "code": "MEMORY_UNAVAILABLE"});
        assert_eq!(
            CannotRun::of_memory_answer(503, &closed),
            Some(CannotRun::StoreUnavailable {
                detail: "store locked".into()
            })
        );
        let bad = json!({"ok": false, "error": "content must not be empty", "code": "BAD_REQUEST"});
        assert_eq!(CannotRun::of_memory_answer(400, &bad), None);
        let absent = CannotRun::of_memory_error("memory is not hosted by this process").unwrap();
        assert!(absent.line().contains("ling status"));
        // A store that answered too late may still land the write: no
        // unavailable store.
        assert_eq!(
            CannotRun::of_memory_error("memory is not reachable — check `ling status`"),
            None
        );
    }
}

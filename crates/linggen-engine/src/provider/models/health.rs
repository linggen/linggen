//! Which models are refusing right now — one board for the whole process.
//!
//! A refusal is read once, where the provider sent it
//! ([`crate::provider::error`]); this board remembers it. The fallback chain
//! steps around a benched model, and `/api/models/health` shows the same
//! board, so what the chain avoids and what the settings page reports are
//! one truth.
//!
//! Process-wide, because a quota belongs to the account and not to one
//! session: a model that is out for the CFO is out for the dream too.
//!
//! Why it exists: until 2026-09-09 nothing read a usage limit's own
//! `resets_at`, and a dream run whose model returned one 503 hopped straight
//! onto a sibling that had been 429ing for hours, spent its last candidate on
//! a guaranteed failure, and died four minutes of work in.

use crate::provider::error::{kind_of, retry_after_of, ProviderErrorKind};
use crate::util::LockExt;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime};

/// How the settings page names a benched model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(any(test, feature = "test-support"), derive(ts_rs::TS), ts(export))]
pub enum ModelHealthStatus {
    Healthy,
    /// Out of its rate limit or quota — it comes back on its own.
    QuotaExhausted,
    /// Failing for another reason (server errors, unreachable).
    Down,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(any(test, feature = "test-support"), derive(ts_rs::TS), ts(export))]
pub struct ModelHealthRecord {
    pub status: ModelHealthStatus,
    /// The refusal behind it.
    pub kind: ProviderErrorKind,
    pub last_error: Option<String>,
    /// Seconds since the model was benched.
    pub since_secs: Option<u64>,
    /// Seconds until it is tried again.
    pub retry_in_secs: Option<u64>,
}

struct Bench {
    kind: ProviderErrorKind,
    since: SystemTime,
    until: SystemTime,
    /// When the provider itself said the model takes requests again — a
    /// usage limit's reset, uncapped. `until` is how long the chain steps
    /// around the model; this is what a caller is told.
    reset_at: Option<SystemTime>,
    last_error: String,
}

/// What the board holds about one model right now.
#[derive(Debug, Clone, PartialEq)]
pub struct Standing {
    pub kind: ProviderErrorKind,
    /// Benched (stepped around by the fallback chain) until then.
    pub until: SystemTime,
    /// The reset the provider named, when it named one.
    pub reset_at: Option<SystemTime>,
}

static BOARD: LazyLock<Mutex<HashMap<String, Bench>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// How long a model stays out when its error named no reset time. Long enough
/// that the next hop of the same chain steps around it, short enough that a
/// one-off blip costs almost nothing.
const BLIND_COOLDOWN: Duration = Duration::from_secs(60);
/// An empty wallet does not refill in a minute.
const QUOTA_COOLDOWN: Duration = Duration::from_secs(30 * 60);
/// A provider that names a date far out should not bench a model for the
/// life of the daemon, and the user can always pick it by hand.
const MAX_COOLDOWN: Duration = Duration::from_secs(60 * 60);

/// Worth trying the next model in the chain for this error.
pub fn is_fallback_worthy_error(err: &anyhow::Error) -> bool {
    kind_of(err).is_fallback_worthy()
}

/// Note that this model just refused, and for how long to leave it alone.
///
/// Only for refusals about the model or its account: a bad request or a
/// missing key is the caller's problem and benching the model would hide it,
/// and a context overflow is about this one conversation, not the model.
pub fn note_unavailable(model_id: &str, err: &anyhow::Error) {
    let kind = kind_of(err);
    if !kind.is_fallback_worthy() || kind == ProviderErrorKind::ContextTooLong {
        return;
    }
    let default = match kind {
        ProviderErrorKind::QuotaExhausted => QUOTA_COOLDOWN,
        _ => BLIND_COOLDOWN,
    };
    let named = retry_after_of(err);
    let wait = named.unwrap_or(default).min(MAX_COOLDOWN);
    let now = SystemTime::now();
    BOARD.lock_ok().insert(
        model_id.to_string(),
        Bench {
            kind,
            since: now,
            until: now + wait,
            reset_at: named.map(|d| now + d),
            last_error: err.to_string(),
        },
    );
}

/// The model just answered: whatever benched it is over.
pub fn note_healthy(model_id: &str) {
    BOARD.lock_ok().remove(model_id);
}

/// Is this model still inside a refusal it told us about? Expired entries are
/// dropped as they are read — nothing else sweeps this map.
pub fn in_cooldown(model_id: &str) -> bool {
    let mut board = BOARD.lock_ok();
    let now = SystemTime::now();
    match board.get(model_id) {
        Some(b) if b.until > now => true,
        Some(b) if b.reset_at.is_some_and(|at| at > now) => false,
        Some(_) => {
            board.remove(model_id);
            false
        }
        None => false,
    }
}

/// Why this model is refusing now, if it is: benched, or past its bench
/// but inside a reset the provider named (a usage limit outlasts the
/// chain's longest bench).
pub fn standing(model_id: &str) -> Option<Standing> {
    let board = BOARD.lock_ok();
    let b = board.get(model_id)?;
    let now = SystemTime::now();
    let live = b.until > now || b.reset_at.is_some_and(|at| at > now);
    live.then(|| Standing {
        kind: b.kind,
        until: b.until,
        reset_at: b.reset_at.filter(|at| *at > now),
    })
}

/// Every model benched right now, for `/api/models/health`.
pub fn health_snapshot() -> Vec<(String, ModelHealthRecord)> {
    let now = SystemTime::now();
    BOARD
        .lock_ok()
        .iter()
        .filter(|(_, b)| b.until > now)
        .map(|(id, b)| {
            let status = match b.kind {
                ProviderErrorKind::RateLimit | ProviderErrorKind::QuotaExhausted => {
                    ModelHealthStatus::QuotaExhausted
                }
                _ => ModelHealthStatus::Down,
            };
            let secs = |d: Result<Duration, _>| d.ok().map(|d: Duration| d.as_secs());
            (
                id.clone(),
                ModelHealthRecord {
                    status,
                    kind: b.kind,
                    last_error: Some(b.last_error.clone()),
                    since_secs: secs(now.duration_since(b.since)),
                    retry_in_secs: secs(b.until.duration_since(now)),
                },
            )
        })
        .collect()
}

/// Test seam — the board is process-wide and tests must not inherit each other.
#[cfg(test)]
pub(crate) fn clear_cooldowns() {
    BOARD.lock_ok().clear();
}

/// Tests that clear the board, or read back what they put on it, take
/// this first, so one never wipes another's entry mid-test.
#[cfg(test)]
pub(crate) fn board_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static GUARD: Mutex<()> = Mutex::new(());
    GUARD.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::error::named_reset;

    fn err(msg: &str) -> anyhow::Error {
        anyhow::anyhow!("{}", msg)
    }

    #[test]
    fn test_rate_limit() {
        assert!(is_fallback_worthy_error(&err(
            "HTTP error (429) Too Many Requests"
        )));
        assert!(is_fallback_worthy_error(&err("rate limit exceeded")));
        // The live openai.rs shape — StatusCode Display puts the reason
        // INSIDE the parens; "(429)" alone never occurs. Regression for the
        // dream run that died un-fallen-back on the team quota.
        assert!(is_fallback_worthy_error(&err(
            "openai error (429 Too Many Requests): {\"error\":{\"type\":\"usage_limit_reached\",\
             \"message\":\"The usage limit has been reached\",\"plan_type\":\"team\"}}"
        )));
    }

    /// The two shapes ChatGPT actually sent on 2026-09-09, verbatim from the
    /// daemon log. Both name a reset; neither was read before this.

    #[test]
    fn a_usage_limit_names_when_it_lifts() {
        let live = "openai error (429 Too Many Requests): {\"error\":{\"type\":\
                    \"usage_limit_reached\",\"message\":\"The usage limit has been \
                    reached\",\"plan_type\":\"team\",\"resets_at\":1788973633,\
                    \"eligible_promo\":null,\"resets_in_seconds\":8973}}";
        // The relative figure wins: it cannot be wrong about our clock.
        assert_eq!(named_reset(live), Some(Duration::from_secs(8973)));

        let absolute_only = format!(
            "openai error (429): {{\"error\":{{\"resets_at\":{}}}}}",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 600
        );
        let secs = named_reset(&absolute_only)
            .expect("a reset in the future")
            .as_secs();
        assert!((595..=600).contains(&secs), "got {secs}s");

        // A reset already past is no reset at all.
        assert_eq!(named_reset("{\"resets_at\":1000}"), None);
        assert_eq!(named_reset("openai error (503): error code: 1102"), None);
    }

    /// A model that refused is stepped around until it said it would be back —
    /// the dream that died on 2026-09-09 hopped onto a sibling that had been
    /// 429ing for hours, because nothing remembered the refusal.

    #[test]
    fn a_model_that_refused_is_benched_until_it_said_it_would_be_back() {
        let _board = board_test_guard();
        clear_cooldowns();
        let quota = err(
            "openai error (429 Too Many Requests): {\"error\":{\"type\":\
             \"usage_limit_reached\",\"resets_in_seconds\":600}}",
        );
        assert!(!in_cooldown("bench-a"));
        note_unavailable("bench-a", &quota);
        assert!(in_cooldown("bench-a"));
        // One model's quota says nothing about another's.
        assert!(!in_cooldown("bench-b"));

        // A blind failure still benches, so the next hop of the same chain
        // steps around it rather than re-trying it immediately.
        note_unavailable(
            "bench-c",
            &err("openai error (503 Service Unavailable): error code: 1102"),
        );
        assert!(in_cooldown("bench-c"));

        // Errors we would not fall back on are the caller's to see, not a
        // reason to hide the model.
        note_unavailable(
            "bench-d",
            &err("openai error (400 Bad Request): malformed tool schema"),
        );
        assert!(!in_cooldown("bench-d"));
    }

    /// An expired bench clears itself on the next read — nothing sweeps the map.

    /// A usage limit's reset is kept as the provider named it, past the
    /// chain's longest bench: the chain may try the model again after an
    /// hour, but a caller asking is told the real reset.
    #[test]
    fn a_usage_limit_keeps_its_named_reset_beyond_the_bench() {
        let _board = board_test_guard();
        let body = r#"{"error":{"type":"usage_limit_reached","resets_in_seconds":8973}}"#;
        note_unavailable("standing-usage", &err(&format!("openai error (429 Too Many Requests): {body}")));
        let s = standing("standing-usage").expect("a usage limit stands");
        assert_eq!(s.kind, ProviderErrorKind::RateLimit);
        let reset = s.reset_at.expect("the provider named a reset");
        let left = reset.duration_since(SystemTime::now()).unwrap().as_secs();
        assert!((8900..=8973).contains(&left), "{left}");
        assert!(s.until < reset, "the bench is capped below the reset");

        // Past the bench, inside the reset: not benched, still standing.
        BOARD.lock_ok().get_mut("standing-usage").unwrap().until = SystemTime::now();
        assert!(!in_cooldown("standing-usage"));
        assert!(standing("standing-usage").is_some());
        note_healthy("standing-usage");
        assert!(standing("standing-usage").is_none());

        // A plain 503 names no reset.
        note_unavailable("standing-down", &err("HTTP error (503) Service Unavailable"));
        assert_eq!(standing("standing-down").unwrap().reset_at, None);
    }

    #[test]
    fn an_expired_bench_lets_the_model_back() {
        let _board = board_test_guard();
        clear_cooldowns();
        BOARD.lock_ok().insert(
            "bench-e".to_string(),
            Bench {
                kind: ProviderErrorKind::RateLimit,
                since: SystemTime::now() - Duration::from_secs(61),
                until: SystemTime::now() - Duration::from_secs(1),
                reset_at: None,
                last_error: String::new(),
            },
        );
        assert!(!in_cooldown("bench-e"));
        assert!(!BOARD.lock_ok().contains_key("bench-e"));
    }

    #[test]
    fn test_context_limit() {
        assert!(is_fallback_worthy_error(&err(
            "HTTP error (400) context length exceeded, max_tokens 8192"
        )));
        assert!(is_fallback_worthy_error(&err(
            "HTTP error (413) content_too_large"
        )));
    }

    #[test]
    fn test_transient_errors() {
        assert!(is_fallback_worthy_error(&err(
            "Model streaming timed out after 60s (no data received)"
        )));
        assert!(is_fallback_worthy_error(&err("request timeout")));
        assert!(is_fallback_worthy_error(&err(
            "HTTP error (502) Bad Gateway"
        )));
        assert!(is_fallback_worthy_error(&err(
            "HTTP error (503) after retries"
        )));
        assert!(is_fallback_worthy_error(&err("connection refused")));
        assert!(is_fallback_worthy_error(&err("connection reset by peer")));
        assert!(is_fallback_worthy_error(&err(
            "dns error: name resolution failed"
        )));
        assert!(is_fallback_worthy_error(&err(
            "connect error: network unreachable"
        )));
    }

    #[test]
    fn test_non_fallback_errors() {
        assert!(!is_fallback_worthy_error(&err("invalid JSON in response")));
        assert!(!is_fallback_worthy_error(&err(
            "HTTP error (401) Unauthorized"
        )));
        assert!(!is_fallback_worthy_error(&err("unknown model")));
    }
}

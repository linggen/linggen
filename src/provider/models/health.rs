//! Which models are refusing right now, and how a provider error is read.

use crate::util::LockExt;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime};
use tokio::sync::RwLock;
use tokio::time::Instant;

// ---------------------------------------------------------------------------
// Model health tracking (in-memory, resets on restart)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelHealthStatus {
    Healthy,
    QuotaExhausted,
    Down,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelHealthRecord {
    pub status: ModelHealthStatus,
    #[serde(skip)]
    pub since: Instant,
    pub last_error: Option<String>,
    /// Seconds since the status changed — populated for API responses.
    pub since_secs: Option<u64>,
}

pub struct ModelHealthTracker {
    records: RwLock<HashMap<String, ModelHealthRecord>>,
}

impl ModelHealthTracker {
    pub fn new() -> Self {
        Self {
            records: RwLock::new(HashMap::new()),
        }
    }

    /// Check if a model is available for use.
    /// QuotaExhausted models become available again after 1 hour.
    /// Down models become available again after 5 minutes.
    pub async fn is_available(&self, model_id: &str) -> bool {
        let records = self.records.read().await;
        let Some(record) = records.get(model_id) else {
            return true; // No record = healthy
        };
        match record.status {
            ModelHealthStatus::Healthy => true,
            ModelHealthStatus::QuotaExhausted => record.since.elapsed().as_secs() > 3600,
            ModelHealthStatus::Down => record.since.elapsed().as_secs() > 300,
        }
    }

    pub async fn mark_error(&self, model_id: &str, error_msg: &str) {
        let status = if is_rate_limit_error_str(error_msg) {
            ModelHealthStatus::QuotaExhausted
        } else {
            ModelHealthStatus::Down
        };
        let mut records = self.records.write().await;
        records.insert(
            model_id.to_string(),
            ModelHealthRecord {
                status,
                since: Instant::now(),
                last_error: Some(error_msg.to_string()),
                since_secs: None,
            },
        );
    }

    pub async fn mark_healthy(&self, model_id: &str) {
        let mut records = self.records.write().await;
        records.remove(model_id);
    }

    pub async fn get_all(&self) -> Vec<(String, ModelHealthRecord)> {
        let records = self.records.read().await;
        records
            .iter()
            .map(|(id, rec)| {
                let mut rec = rec.clone();
                rec.since_secs = Some(rec.since.elapsed().as_secs());
                (id.clone(), rec)
            })
            .collect()
    }
}

/// Check if an error message string indicates a rate limit (HTTP 429).
fn is_rate_limit_error_str(msg: &str) -> bool {
    msg.contains("(429)")
        || msg.to_lowercase().contains("rate limit")
        || msg.to_lowercase().contains("quota")
}

// ---------------------------------------------------------------------------
// Error classification for fallback routing
// ---------------------------------------------------------------------------

pub fn is_rate_limit_error(err: &anyhow::Error) -> bool {
    // NOTE: provider errors format the status as `({})` with reqwest's
    // StatusCode, whose Display includes the canonical reason — the live
    // string is "(429 Too Many Requests)", never "(429)". Match the prefix.
    let msg = err.to_string().to_lowercase();
    msg.contains("(429")
        || msg.contains("rate limit")
        || msg.contains("too many requests")
        || msg.contains("usage_limit_reached")
}

/// Check if an error indicates a context/token limit exceeded (HTTP 400 + context keywords).
pub fn is_context_limit_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_lowercase();
    (msg.contains("(400") || msg.contains("(413"))
        && (msg.contains("context")
            || msg.contains("token")
            || msg.contains("too long")
            || msg.contains("max_tokens")
            || msg.contains("content_too_large"))
}

/// Returns true if the error indicates a transient connectivity or availability issue.
fn is_transient_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("timed out")
        || msg.contains("timeout")
        || msg.contains("(502")
        || msg.contains("(503")
        || msg.contains("connection refused")
        || msg.contains("connection reset")
        || msg.contains("dns error")
        || msg.contains("connect error")
}

/// Returns true if the error is a rate limit, context limit, or transient failure
/// that warrants trying another model.
pub fn is_fallback_worthy_error(err: &anyhow::Error) -> bool {
    is_rate_limit_error(err) || is_context_limit_error(err) || is_transient_error(err)
}

/// Models that told us they are unavailable, and when they said they'd be back.
///
/// A usage-limit error carries its own `resets_at`, and nothing read it until
/// 2026-09-09: a dream run whose model returned one 503 hopped straight onto a
/// sibling that had been 429ing for hours, spent its last candidate on a
/// guaranteed failure, and died four minutes of work in — reporting the
/// sibling's quota error, which had nothing to do with why it stopped.
///
/// Process-wide, because a quota belongs to the account and not to one
/// session: a model that is out for the CFO is out for the dream too.
static COOLDOWNS: LazyLock<Mutex<HashMap<String, SystemTime>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// How long a model stays out when its error named no reset time. Long enough
/// that the next hop of the same chain steps around it, short enough that a
/// one-off blip costs almost nothing.
const BLIND_COOLDOWN: Duration = Duration::from_secs(60);

/// The reset time a provider named in its own error, as seconds from now.
/// `resets_in_seconds` is preferred over `resets_at` — a relative figure
/// cannot be wrong about our clock.
fn named_reset(msg: &str) -> Option<Duration> {
    fn number_after(msg: &str, key: &str) -> Option<u64> {
        let at = msg.find(key)? + key.len();
        let rest = msg[at..].trim_start_matches([':', ' ', '"']);
        let end = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        rest[..end].parse().ok()
    }
    if let Some(secs) = number_after(msg, "\"resets_in_seconds\"") {
        return Some(Duration::from_secs(secs));
    }
    let at = number_after(msg, "\"resets_at\"")?;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();
    (at > now).then(|| Duration::from_secs(at - now))
}

/// Note that this model just refused, and for how long to leave it alone.
///
/// Only for errors we would fall back on: a bad request or a missing key is
/// the caller's problem and benching the model would hide it.
pub fn note_unavailable(model_id: &str, err: &anyhow::Error) {
    if !is_fallback_worthy_error(err) {
        return;
    }
    let wait = named_reset(&err.to_string()).unwrap_or(BLIND_COOLDOWN);
    // Cap it: a provider that names a date far out should not bench a model
    // for the life of the daemon, and the user can always pick it by hand.
    let wait = wait.min(Duration::from_secs(60 * 60));
    COOLDOWNS
        .lock_ok()
        .insert(model_id.to_string(), SystemTime::now() + wait);
}

/// Is this model still inside a refusal it told us about? Expired entries are
/// dropped as they are read — nothing else sweeps this map.
pub fn in_cooldown(model_id: &str) -> bool {
    let mut map = COOLDOWNS.lock_ok();
    match map.get(model_id) {
        Some(until) if *until > SystemTime::now() => true,
        Some(_) => {
            map.remove(model_id);
            false
        }
        None => false,
    }
}

/// Test seam — the map is process-wide and tests must not inherit each other.
#[cfg(test)]
pub(crate) fn clear_cooldowns() {
    COOLDOWNS.lock_ok().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn an_expired_bench_lets_the_model_back() {
        clear_cooldowns();
        COOLDOWNS.lock_ok().insert(
            "bench-e".to_string(),
            SystemTime::now() - Duration::from_secs(1),
        );
        assert!(!in_cooldown("bench-e"));
        assert!(!COOLDOWNS.lock_ok().contains_key("bench-e"));
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

//! What a provider's refusal means, decided once where the reply arrives.
//!
//! Every client turns a non-success response into a [`ProviderError`] at its
//! HTTP boundary: the status and body are read there, while they are still a
//! status and a body. Everything downstream — the fallback chain, the model
//! health board, the chat's error line, telemetry — asks [`kind_of`] instead
//! of searching the error's text.
//!
//! The text is unchanged: a `ProviderError` displays exactly the message the
//! clients produced before, so the markers the UI reads (`AUTH_REQUIRED:`,
//! `BILLING_REQUIRED:`) and the log lines stay as they were.

use serde::Serialize;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum ProviderErrorKind {
    /// Too many requests, or a plan's usage window is used up — it lifts on
    /// its own (often at a time the provider names).
    RateLimit,
    /// Out of money or credit: a 402, OpenAI's `insufficient_quota`.
    QuotaExhausted,
    /// Missing, expired or refused credentials.
    Auth,
    /// The request is bigger than the model's window.
    ContextTooLong,
    /// The provider rejected the request itself (bad schema, unknown model).
    BadRequest,
    /// The provider failed or is overloaded (5xx, Anthropic's 529).
    Server5xx,
    /// We never got a whole answer: timeout, refused/reset connection, DNS,
    /// a stream that went silent.
    Network,
    /// The model answered with nothing at all.
    EmptyResponse,
    Other,
}

impl ProviderErrorKind {
    /// Worth trying the next model in the chain: the refusal is about this
    /// model or its account right now, not about the request. A bad request
    /// or a missing key is the caller's to see — another model would only
    /// hide it.
    pub fn is_fallback_worthy(self) -> bool {
        matches!(
            self,
            Self::RateLimit
                | Self::QuotaExhausted
                | Self::ContextTooLong
                | Self::Server5xx
                | Self::Network
                | Self::EmptyResponse
        )
    }
}

#[derive(Debug)]
pub struct ProviderError {
    pub kind: ProviderErrorKind,
    /// When the provider said it would take requests again.
    pub retry_after: Option<Duration>,
    message: String,
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ProviderError {}

impl ProviderError {
    /// A non-success HTTP reply, classified from its status and body.
    /// `message` is the text the user and the log see.
    pub fn http(status: reqwest::StatusCode, body: &str, message: String) -> Self {
        Self {
            kind: classify_http(status.as_u16(), body),
            retry_after: named_reset(body),
            message,
        }
    }

    /// A refusal whose kind the caller already knows (a sign-in CTA, a
    /// billing card).
    pub fn new(kind: ProviderErrorKind, message: String) -> Self {
        Self {
            kind,
            retry_after: None,
            message,
        }
    }

    /// An error the provider sent inside an open stream (an SSE `error`
    /// event): no status, only words.
    pub fn stream_event(message: String) -> Self {
        Self {
            kind: classify_message(&message),
            retry_after: named_reset(&message),
            message,
        }
    }
}

/// What kind of refusal this error is. A [`ProviderError`] anywhere in the
/// chain answers; so does a transport error from reqwest or the socket.
/// Anything else — an error that crossed a process boundary as text (a
/// proxy room, an old session) — is read from its words, once, here.
pub fn kind_of(err: &anyhow::Error) -> ProviderErrorKind {
    for cause in err.chain() {
        if let Some(p) = cause.downcast_ref::<ProviderError>() {
            return p.kind;
        }
        if let Some(r) = cause.downcast_ref::<reqwest::Error>() {
            if r.is_timeout() || r.is_connect() || r.is_request() || r.is_body() {
                return ProviderErrorKind::Network;
            }
        }
        if cause.downcast_ref::<std::io::Error>().is_some() {
            return ProviderErrorKind::Network;
        }
    }
    classify_message(&format!("{err:#}"))
}

/// When the provider said it would be back, if it said.
pub fn retry_after_of(err: &anyhow::Error) -> Option<Duration> {
    err.chain()
        .find_map(|c| c.downcast_ref::<ProviderError>())
        .map(|p| p.retry_after)
        .unwrap_or_else(|| named_reset(&format!("{err:#}")))
}

/// [`kind_of`] for an error that only survived as its text — a failed run's
/// message, say.
pub fn kind_of_message(msg: &str) -> ProviderErrorKind {
    classify_message(msg)
}

fn classify_http(status: u16, body: &str) -> ProviderErrorKind {
    use ProviderErrorKind::*;
    let b = body.to_lowercase();
    match status {
        401 | 403 => Auth,
        402 => QuotaExhausted,
        413 => ContextTooLong,
        429 if b.contains("insufficient_quota") || b.contains("exceeded your current quota") => {
            QuotaExhausted
        }
        429 => RateLimit,
        400 | 404 | 422 if names_context_limit(&b) => ContextTooLong,
        400 | 404 | 422 => BadRequest,
        500..=599 => Server5xx,
        _ => Other,
    }
}

fn names_context_limit(lower: &str) -> bool {
    lower.contains("context")
        || lower.contains("token")
        || lower.contains("too long")
        || lower.contains("max_tokens")
        || lower.contains("content_too_large")
}

/// The status a client wrote into its message — `"openai error (429 Too Many
/// Requests): …"`, `"HTTP error (502) …"`.
fn status_in(msg: &str) -> Option<u16> {
    let open = msg.find('(')?;
    let digits: String = msg[open + 1..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    (digits.len() == 3).then(|| digits.parse().ok()).flatten()
}

/// The one reading of an error's words, for errors that arrive as text.
fn classify_message(msg: &str) -> ProviderErrorKind {
    use ProviderErrorKind::*;
    let lower = msg.to_lowercase();
    if msg.contains("AUTH_REQUIRED") {
        return Auth;
    }
    if msg.contains("BILLING_REQUIRED") {
        return QuotaExhausted;
    }
    if let Some(status) = status_in(msg) {
        return classify_http(status, msg);
    }
    if lower.contains("insufficient_quota") {
        QuotaExhausted
    } else if lower.contains("rate limit")
        || lower.contains("too many requests")
        || lower.contains("usage_limit_reached")
    {
        RateLimit
    } else if lower.contains("overloaded") {
        Server5xx
    } else if lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("connection refused")
        || lower.contains("connection reset")
        || lower.contains("connection closed")
        || lower.contains("connection dropped")
        || lower.contains("dns error")
        || lower.contains("connect error")
        || lower.contains("stream error")
    {
        Network
    } else if lower.contains("empty response") {
        EmptyResponse
    } else {
        Other
    }
}

/// The reset time a provider named in its own error, as time from now.
/// `resets_in_seconds` is preferred over `resets_at` — a relative figure
/// cannot be wrong about our clock.
pub(crate) fn named_reset(msg: &str) -> Option<Duration> {
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

#[cfg(test)]
mod tests {
    use super::ProviderErrorKind::*;
    use super::*;
    use reqwest::StatusCode;

    fn http(status: u16, body: &str) -> anyhow::Error {
        let status = StatusCode::from_u16(status).unwrap();
        ProviderError::http(status, body, format!("provider error ({status}): {body}")).into()
    }

    #[test]
    fn openai_429s_split_into_a_rate_limit_and_an_empty_wallet() {
        // A plain rate limit lifts on its own.
        let rate = http(
            429,
            r#"{"error":{"message":"Rate limit reached for gpt-4.1 in organization org-x on tokens per min (TPM): Limit 30000, Used 29000, Requested 2000.","type":"tokens","code":"rate_limit_exceeded"}}"#,
        );
        assert_eq!(kind_of(&rate), RateLimit);
        // insufficient_quota is billing — a 429 by status, a quota by meaning.
        let quota = http(
            429,
            r#"{"error":{"message":"You exceeded your current quota, please check your plan and billing details.","type":"insufficient_quota","code":"insufficient_quota"}}"#,
        );
        assert_eq!(kind_of(&quota), QuotaExhausted);
    }

    #[test]
    fn the_chatgpt_usage_limit_is_a_rate_limit_with_its_reset() {
        let err = http(
            429,
            r#"{"error":{"type":"usage_limit_reached","message":"The usage limit has been reached","plan_type":"team","resets_at":1788973633,"eligible_promo":null,"resets_in_seconds":8973}}"#,
        );
        assert_eq!(kind_of(&err), RateLimit);
        assert_eq!(retry_after_of(&err), Some(Duration::from_secs(8973)));
        assert!(kind_of(&err).is_fallback_worthy());
    }

    #[test]
    fn anthropic_overloaded_is_a_server_failure_either_way_it_arrives() {
        let status = http(
            529,
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        );
        assert_eq!(kind_of(&status), Server5xx);
        // Mid-stream it comes as an SSE error event: words only.
        let event: anyhow::Error = ProviderError::stream_event("Overloaded".into()).into();
        assert_eq!(kind_of(&event), Server5xx);
        assert!(kind_of(&event).is_fallback_worthy());
    }

    #[test]
    fn a_deepseek_402_is_an_empty_wallet() {
        let err = http(
            402,
            r#"{"error":{"message":"Insufficient Balance","type":"unknown_error","param":null,"code":"invalid_request_error"}}"#,
        );
        assert_eq!(kind_of(&err), QuotaExhausted);
    }

    #[test]
    fn a_context_overflow_is_named_as_one() {
        let openai = http(
            400,
            r#"{"error":{"message":"This model's maximum context length is 128000 tokens. However, your messages resulted in 131072 tokens.","type":"invalid_request_error","code":"context_length_exceeded"}}"#,
        );
        assert_eq!(kind_of(&openai), ContextTooLong);
        let anthropic = http(
            400,
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 215000 tokens > 200000 maximum"}}"#,
        );
        assert_eq!(kind_of(&anthropic), ContextTooLong);
        assert_eq!(kind_of(&http(413, "content_too_large")), ContextTooLong);
    }

    #[test]
    fn a_bad_request_and_a_bad_key_are_the_callers_to_see() {
        let schema = http(
            400,
            r#"{"error":{"message":"Invalid schema for function 'Show'"}}"#,
        );
        assert_eq!(kind_of(&schema), BadRequest);
        assert!(!kind_of(&schema).is_fallback_worthy());
        let key = http(401, r#"{"error":{"message":"Incorrect API key provided"}}"#);
        assert_eq!(kind_of(&key), Auth);
        assert!(!kind_of(&key).is_fallback_worthy());
    }

    #[test]
    fn words_alone_are_read_once_the_same_way() {
        let text = |m: &str| kind_of(&anyhow::anyhow!("{m}"));
        assert_eq!(
            text("openai error (429 Too Many Requests): {\"error\":{}}"),
            RateLimit
        );
        assert_eq!(text("HTTP error (502) Bad Gateway"), Server5xx);
        assert_eq!(text("HTTP error (401) Unauthorized"), Auth);
        assert_eq!(
            text("AUTH_REQUIRED: ChatGPT session expired. Sign in with ChatGPT to continue."),
            Auth
        );
        assert_eq!(
            text("BILLING_REQUIRED: Your free tokens are used up"),
            QuotaExhausted
        );
        assert_eq!(
            text("stream read timed out (backend went silent mid-stream): x"),
            Network
        );
        assert_eq!(text("Proxy connection closed"), Network);
        assert_eq!(text("invalid JSON in response"), Other);
    }

    #[test]
    fn the_message_is_what_the_client_wrote() {
        let err = http(429, "{}");
        assert_eq!(
            err.to_string(),
            "provider error (429 Too Many Requests): {}"
        );
    }
}

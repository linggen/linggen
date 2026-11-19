//! Provider implementations + model dispatch.
//!
//! Wire-format clients (`anthropic`, `ollama`, `openai`) map the engine's
//! generic [`crate::message::ChatMessage`] to one provider's
//! chat-completions API. `models` defines the shared types they speak
//! (`ChatRequest`, `StreamChunk`, `TokenUsage`, `ToolCallChunk`) plus
//! `ModelManager` — the multi-provider dispatcher (default models +
//! fallback chains). `proxy_provider` is the dispatcher variant used for
//! proxy/relay sessions.

pub mod anthropic;
pub mod claude_auth;
pub mod codex_auth;
pub mod error;
pub mod models;
pub mod ollama;
pub mod openai;
pub mod proxy_provider;

/// Render a mid-stream body read error with a truthful name: reqwest wraps
/// a `read_timeout` firing between chunks as "error decoding response body",
/// which the transient/fallback classifiers can't recognize. A silent
/// backend must trip the model fallback chain, not read as a decode bug
/// (observed live 2026-07-10: a 35-minute wedge on a stream that never sent
/// a byte after headers).
pub(crate) fn stream_read_error(e: impl std::fmt::Display) -> anyhow::Error {
    let msg = e.to_string();
    let msg = if msg.contains("decoding response body") || msg.contains("timed out") {
        format!("stream read timed out (backend went silent mid-stream): {msg}")
    } else {
        format!("stream error: {msg}")
    };
    error::ProviderError::new(error::ProviderErrorKind::Network, msg).into()
}

/// The value of one SSE line's `field` (`data`, `event`), or `None` for a
/// line of another field. SSE allows the space after the colon or none.
pub(crate) fn sse_field<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(field)?.strip_prefix(':')?;
    Some(rest.trim_start())
}

/// Seconds of total silence the first answer of a call may take: no response
/// headers and no first stream event. Holds `[routing].first_event_timeout_secs`
/// (the built-in default when that is unset); `0` means the guard is off.
static FIRST_EVENT_SECS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(DEFAULT_FIRST_EVENT_SECS);
const DEFAULT_FIRST_EVENT_SECS: u64 = 30;
const FIRST_EVENT_ENV: &str = "LINGGEN_FIRST_EVENT_TIMEOUT_SECS";

/// Apply `[routing].first_event_timeout_secs` (`None`: the default).
pub fn set_first_event_timeout(secs: Option<u64>) {
    FIRST_EVENT_SECS.store(
        secs.unwrap_or(DEFAULT_FIRST_EVENT_SECS),
        std::sync::atomic::Ordering::Relaxed,
    );
}

fn first_event_window() -> Option<std::time::Duration> {
    let secs = std::env::var(FIRST_EVENT_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or_else(|| FIRST_EVENT_SECS.load(std::sync::atomic::Ordering::Relaxed));
    (secs > 0).then(|| std::time::Duration::from_secs(secs))
}

/// One call's first-answer deadline, measured from [`FirstEvent::start`]: the
/// request, its response headers and the stream's first event all fit in one
/// window. The wire's own `read_timeout` (180 s) bounds silence *between*
/// reads; this bounds the wait for the first one — the stall that took 164 s
/// on a 68-token reply (Lingjing, 2026-10-22). A reasoning model keeps
/// sending reasoning and `created` events while it thinks, so any event
/// ends the guard — only a backend with nothing to say trips it.
pub(crate) struct FirstEvent {
    window: Option<std::time::Duration>,
    deadline: Option<tokio::time::Instant>,
}

impl FirstEvent {
    /// The guard for one call to `base_url`. A backend on this machine
    /// (Ollama, MLX, LM Studio) is exempt: its silence is a model loading or
    /// a long prompt being read, and leaving it for the chain would leave
    /// the machine.
    pub(crate) fn start(base_url: &str) -> Self {
        let window = first_event_window().filter(|_| !is_local_backend(base_url));
        Self {
            window,
            deadline: window.map(|w| tokio::time::Instant::now() + w),
        }
    }

    /// `fut` (a request's send, or the first read of its body) under the
    /// deadline. Past it, a `Stalled` error the engine can retry.
    pub(crate) async fn within<T>(
        &self,
        fut: impl std::future::Future<Output = T>,
    ) -> anyhow::Result<T> {
        let (Some(deadline), Some(window)) = (self.deadline, self.window) else {
            return Ok(fut.await);
        };
        tokio::time::timeout_at(deadline, fut).await.map_err(|_| {
            error::ProviderError::new(
                error::ProviderErrorKind::Stalled,
                format!(
                    "no response from the model in {}s (first-event guard)",
                    window.as_secs()
                ),
            )
            .into()
        })
    }
}

/// Whether `base_url` names a local server: this machine (`localhost`, a
/// loopback address) or this network (a private or link-local address, a
/// `.local` name). The one answer for the first-event guard's patience and
/// for readiness's "needs no key".
pub(crate) fn is_local_backend(base_url: &str) -> bool {
    let rest = base_url.split_once("://").map(|(_, r)| r).unwrap_or(base_url);
    let authority = rest.split('/').next().unwrap_or("");
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => authority.rsplit_once(':').map(|(h, _)| h).unwrap_or(authority),
    };
    let host = host.to_ascii_lowercase();
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
mod first_event_tests {
    use super::*;
    use std::time::Duration;

    fn guard(ms: Option<u64>) -> FirstEvent {
        let window = ms.map(Duration::from_millis);
        FirstEvent {
            window,
            deadline: window.map(|w| tokio::time::Instant::now() + w),
        }
    }

    #[tokio::test]
    async fn a_silent_call_stalls_at_the_window() {
        let err = guard(Some(50))
            .within(std::future::pending::<()>())
            .await
            .unwrap_err();
        assert_eq!(error::kind_of(&err), error::ProviderErrorKind::Stalled);
    }

    #[tokio::test]
    async fn a_prompt_answer_passes_and_a_zero_window_is_off() {
        assert_eq!(guard(Some(500)).within(async { 7 }).await.unwrap(), 7);
        let slow = async {
            tokio::time::sleep(Duration::from_millis(80)).await;
            9
        };
        assert_eq!(guard(None).within(slow).await.unwrap(), 9);
    }

    #[tokio::test]
    async fn the_send_and_the_first_read_share_one_window() {
        let g = guard(Some(120));
        g.within(tokio::time::sleep(Duration::from_millis(80)))
            .await
            .unwrap();
        let err = g
            .within(tokio::time::sleep(Duration::from_millis(80)))
            .await
            .unwrap_err();
        assert_eq!(error::kind_of(&err), error::ProviderErrorKind::Stalled);
    }

    #[test]
    fn a_local_backend_is_exempt_from_the_guard() {
        for local in [
            "http://127.0.0.1:11434/v1",
            "http://localhost:8080",
            "http://LOCALHOST:1234/v1",
            "http://[::1]:11434",
        ] {
            assert!(is_local_backend(local), "{local}");
            assert!(FirstEvent::start(local).window.is_none(), "{local}");
        }
        for remote in [
            "https://api.openai.com/v1",
            "https://linggen.dev/api/llm",
            "not a url",
        ] {
            assert!(!is_local_backend(remote), "{remote}");
        }
    }

    /// A server on this network (LM Studio, MLX, Ollama's `/v1` on another
    /// Mac) loading a model gets the same patience as one on this machine.
    #[test]
    fn a_server_on_this_network_is_local_too() {
        for lan in [
            "http://192.168.1.20:11434/v1",
            "http://10.0.0.5:1234/v1",
            "http://172.16.3.4:8080",
            "http://169.254.10.10:8000",
            "http://studio.local:1234/v1",
            "http://Studio.LOCAL:1234/v1",
        ] {
            assert!(is_local_backend(lan), "{lan}");
            assert!(FirstEvent::start(lan).window.is_none(), "{lan}");
        }
        for remote in ["http://8.8.8.8/v1", "https://example.com/v1", "http://172.32.0.1"] {
            assert!(!is_local_backend(remote), "{remote}");
        }
    }

    #[test]
    fn a_stall_read_from_its_words_is_still_a_stall() {
        let err = anyhow::anyhow!("no response from the model in 30s (first-event guard)");
        assert_eq!(error::kind_of(&err), error::ProviderErrorKind::Stalled);
    }
}

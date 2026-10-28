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

/// Whether `base_url` names this machine: `localhost` or a loopback address.
pub(crate) fn is_local_backend(base_url: &str) -> bool {
    let Ok(url) = url::Url::parse(base_url) else {
        return false;
    };
    match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
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
            "http://192.168.1.20:11434",
            "not a url",
        ] {
            assert!(!is_local_backend(remote), "{remote}");
        }
    }

    #[test]
    fn a_stall_read_from_its_words_is_still_a_stall() {
        let err = anyhow::anyhow!("no response from the model in 30s (first-event guard)");
        assert_eq!(error::kind_of(&err), error::ProviderErrorKind::Stalled);
    }
}

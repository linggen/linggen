//! Hugging Face, with hf-mirror.com behind it for regions where
//! huggingface.co is blocked (mainland China). Same rule as
//! [`crate::mirror`]: the mirror is a fallback, never a second opinion — a
//! connect error, timeout or 5xx moves on, a 404 is final.
//!
//! A user-set `HF_ENDPOINT` always wins and is never overridden. Otherwise
//! huggingface.co is probed, and re-probed every [`PROBE_TTL`] so a VPN
//! switched on or off is noticed. The engine never writes the variable into
//! its own environment (`set_var` races every other thread reading it):
//! each spawn of something that downloads from the Hub passes
//! `.env("HF_ENDPOINT", endpoint().await)` itself.

use anyhow::Result;
use std::time::{Duration, Instant};

pub const HF: &str = "https://huggingface.co";
pub const HF_MIRROR: &str = "https://hf-mirror.com";

/// How long a probe's answer stands.
const PROBE_TTL: Duration = Duration::from_secs(10 * 60);

/// What the probe settled.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Resolved {
    endpoint: String,
    /// The user named the endpoint — use it alone, no fallback.
    user_set: bool,
}

static PROBED: std::sync::Mutex<Option<(Instant, Resolved)>> = std::sync::Mutex::new(None);
static PROBING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The user's `HF_ENDPOINT`, read once — the in-process voice loader may
/// later set the variable for its own library, which is not the user's.
fn user_endpoint() -> Option<String> {
    static USER: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    USER.get_or_init(|| {
        std::env::var("HF_ENDPOINT")
            .ok()
            .map(|v| v.trim().trim_end_matches('/').to_string())
            .filter(|v| !v.is_empty())
    })
    .clone()
}

fn choose(user: Option<String>, hf_reachable: bool) -> Resolved {
    match user {
        Some(endpoint) => Resolved {
            endpoint,
            user_set: true,
        },
        None => Resolved {
            endpoint: if hf_reachable { HF } else { HF_MIRROR }.to_string(),
            user_set: false,
        },
    }
}

fn fresh(at: Instant, r: &Resolved, now: Instant) -> Option<Resolved> {
    (now.duration_since(at) < PROBE_TTL).then(|| r.clone())
}

fn cached() -> Option<Resolved> {
    let guard = PROBED.lock().ok()?;
    let (at, r) = guard.as_ref()?;
    fresh(*at, r, Instant::now())
}

/// Probe huggingface.co (one prober at a time; the rest wait for its answer).
async fn probe() -> Resolved {
    let _probing = PROBING.lock().await;
    if let Some(r) = cached() {
        return r;
    }
    let r = choose(None, crate::reach::reachable(HF).await);
    if r.endpoint == HF_MIRROR {
        tracing::warn!("[hf] huggingface.co unreachable; using {HF_MIRROR}");
    }
    if let Ok(mut guard) = PROBED.lock() {
        *guard = Some((Instant::now(), r.clone()));
    }
    r
}

async fn resolved() -> Resolved {
    if let Some(user) = user_endpoint() {
        return choose(Some(user), true);
    }
    match cached() {
        Some(r) => r,
        None => probe().await,
    }
}

/// The Hugging Face endpoint to use. Pass it as `HF_ENDPOINT` to anything
/// spawned that downloads from the Hub.
pub async fn endpoint() -> String {
    resolved().await.endpoint
}

/// The Hub addresses to try, in order: the user's endpoint alone, the
/// mirror alone when huggingface.co is unreachable, else huggingface.co with
/// the mirror behind it.
fn bases_of(r: &Resolved) -> Vec<String> {
    if r.user_set || r.endpoint != HF {
        vec![r.endpoint.clone()]
    } else {
        vec![HF.to_string(), HF_MIRROR.to_string()]
    }
}

/// Where to fetch `path` (e.g. `hexgrad/Kokoro-82M/resolve/main/x.pt`)
/// from `bases`, in order.
fn urls_at(bases: &[String], path: &str) -> Vec<String> {
    let path = path.trim_start_matches('/');
    bases.iter().map(|base| format!("{base}/{path}")).collect()
}

/// The Hub addresses a fetch may go to now, in order. A caller that makes
/// several GETs asks once and passes the list to [`get_bytes_at`] and
/// [`download_at`], so every file of one model comes from one decision.
pub async fn bases() -> Vec<String> {
    bases_of(&resolved().await)
}

/// Did the user name the endpoint (`HF_ENDPOINT`)? Then there is no mirror
/// behind it, and what it refuses is its owner's word, not the Hub's weather.
pub async fn endpoint_is_users() -> bool {
    resolved().await.user_set
}

/// GET a Hub file by its path, huggingface.co first and hf-mirror.com
/// second (or the user's endpoint alone).
pub async fn get_bytes(path: &str) -> Result<Vec<u8>> {
    get_bytes_at(&bases().await, path).await
}

/// [`get_bytes`] against `bases` (from [`bases`]).
pub async fn get_bytes_at(bases: &[String], path: &str) -> Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .connect_timeout(crate::mirror::CONNECT_TIMEOUT)
        .build()?;
    crate::mirror::get_bytes_any(&client, &urls_at(bases, path)).await
}

/// GET a Hub file into `part`, resuming what is already there (a file too
/// big to hold in memory), with the same rule as [`get_bytes`]: a source that
/// is unreachable or unwell hands over to the next, a definite refusal (a
/// 4xx other than 408/429) is final. `part` holds a whole file only when
/// this returns Ok; the caller verifies it before it is used.
pub async fn download_at(bases: &[String], path: &str, part: &std::path::Path) -> Result<()> {
    let mut last = None;
    for url in urls_at(bases, path) {
        match crate::runtime::download_resumable(&url, part).await {
            Ok(()) => return Ok(()),
            Err(e) if is_final(&e) => return Err(e),
            Err(e) => {
                tracing::warn!("[hf] {url}: {e:#}");
                last = Some(e);
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no source to fetch")))
}

fn is_final(e: &anyhow::Error) -> bool {
    let Some(status) = e.downcast_ref::<reqwest::Error>().and_then(|r| r.status()) else {
        return false;
    };
    status.is_client_error()
        && status != reqwest::StatusCode::TOO_MANY_REQUESTS
        && status != reqwest::StatusCode::REQUEST_TIMEOUT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_endpoint_wins_and_stands_alone() {
        let r = choose(Some("https://my.hub".into()), false);
        assert!(r.user_set);
        assert_eq!(urls_at(&bases_of(&r), "a/b"), vec!["https://my.hub/a/b"]);
    }

    #[test]
    fn reachable_hub_keeps_the_mirror_as_fallback() {
        let r = choose(None, true);
        assert_eq!(
            urls_at(&bases_of(&r), "/o/r/resolve/main/v.pt"),
            vec![
                "https://huggingface.co/o/r/resolve/main/v.pt",
                "https://hf-mirror.com/o/r/resolve/main/v.pt",
            ]
        );
    }

    #[test]
    fn a_probe_stands_for_ten_minutes_then_is_asked_again() {
        let r = choose(None, false);
        let at = Instant::now();
        assert_eq!(fresh(at, &r, at + Duration::from_secs(60)), Some(r.clone()));
        assert_eq!(fresh(at, &r, at + PROBE_TTL), None);
    }

    #[test]
    fn unreachable_hub_goes_straight_to_the_mirror() {
        let r = choose(None, false);
        assert_eq!(r.endpoint, HF_MIRROR);
        assert_eq!(
            urls_at(&bases_of(&r), "o/r/resolve/main/v.pt"),
            vec!["https://hf-mirror.com/o/r/resolve/main/v.pt"]
        );
    }
}

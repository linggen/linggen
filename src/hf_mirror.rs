//! Hugging Face, with hf-mirror.com behind it for regions where
//! huggingface.co is blocked (mainland China). Same rule as
//! [`crate::mirror`]: the mirror is a fallback, never a second opinion — a
//! connect error, timeout or 5xx moves on, a 404 is final.
//!
//! A user-set `HF_ENDPOINT` always wins and is never overridden. Otherwise
//! huggingface.co is probed once per process; when it is unreachable the
//! engine exports `HF_ENDPOINT=https://hf-mirror.com`, so everything that
//! inherits the environment — the Python runtime (mlx-audio, snapshot
//! downloads), skill tools, the in-process `hf-hub`/`any-tts` loaders —
//! goes through the mirror with no setup.

use anyhow::Result;
use tokio::sync::OnceCell;

pub const HF: &str = "https://huggingface.co";
pub const HF_MIRROR: &str = "https://hf-mirror.com";

/// What the probe settled for this process.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Resolved {
    endpoint: String,
    /// The user named the endpoint — use it alone, no fallback.
    user_set: bool,
}

static RESOLVED: OnceCell<Resolved> = OnceCell::const_new();

fn user_endpoint() -> Option<String> {
    std::env::var("HF_ENDPOINT")
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
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

async fn resolved() -> &'static Resolved {
    RESOLVED
        .get_or_init(|| async {
            let user = user_endpoint();
            let hf_ok = user.is_some() || crate::reach::reachable(HF).await;
            let r = choose(user, hf_ok);
            if !r.user_set && r.endpoint == HF_MIRROR {
                tracing::warn!("[hf] huggingface.co unreachable; using {HF_MIRROR}");
                std::env::set_var("HF_ENDPOINT", HF_MIRROR);
            }
            r
        })
        .await
}

/// The Hugging Face endpoint to use (probing once, and exporting
/// `HF_ENDPOINT` when the mirror is chosen). Call before anything that
/// downloads from the Hub or spawns a process that will.
pub async fn endpoint() -> &'static str {
    &resolved().await.endpoint
}

/// Where to fetch `path` (e.g. `hexgrad/Kokoro-82M/resolve/main/x.pt`),
/// in order.
fn sources_for(r: &Resolved, path: &str) -> Vec<String> {
    let path = path.trim_start_matches('/');
    let bases: Vec<&str> = if r.user_set || r.endpoint != HF {
        vec![&r.endpoint]
    } else {
        vec![HF, HF_MIRROR]
    };
    bases.iter().map(|base| format!("{base}/{path}")).collect()
}

/// GET a Hub file by its path, huggingface.co first and hf-mirror.com
/// second (or the user's endpoint alone).
pub async fn get_bytes(path: &str) -> Result<Vec<u8>> {
    let urls = sources_for(resolved().await, path);
    let client = reqwest::Client::builder()
        .connect_timeout(crate::mirror::CONNECT_TIMEOUT)
        .build()?;
    crate::mirror::get_bytes_any(&client, &urls).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_endpoint_wins_and_stands_alone() {
        let r = choose(Some("https://my.hub".into()), false);
        assert!(r.user_set);
        assert_eq!(sources_for(&r, "a/b"), vec!["https://my.hub/a/b"]);
    }

    #[test]
    fn reachable_hub_keeps_the_mirror_as_fallback() {
        let r = choose(None, true);
        assert_eq!(
            sources_for(&r, "/o/r/resolve/main/v.pt"),
            vec![
                "https://huggingface.co/o/r/resolve/main/v.pt",
                "https://hf-mirror.com/o/r/resolve/main/v.pt",
            ]
        );
    }

    #[test]
    fn unreachable_hub_goes_straight_to_the_mirror() {
        let r = choose(None, false);
        assert_eq!(r.endpoint, HF_MIRROR);
        assert_eq!(
            sources_for(&r, "o/r/resolve/main/v.pt"),
            vec!["https://hf-mirror.com/o/r/resolve/main/v.pt"]
        );
    }
}

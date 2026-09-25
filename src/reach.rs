//! Can this machine reach a host? The one reachability probe every
//! fallback shares (Hugging Face → hf-mirror.com, the region facts), so
//! "unreachable" means the same thing everywhere: no connection, a
//! timeout, or a 5xx. Any other answer — even a 404 — is the host talking.

use std::time::Duration;

/// Short, like [`crate::mirror::CONNECT_TIMEOUT`]: a blackholed host should
/// cost seconds, not the default minutes.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether `url` answers within [`PROBE_TIMEOUT`].
pub async fn reachable(url: &str) -> bool {
    reachable_within(url, PROBE_TIMEOUT).await
}

pub async fn reachable_within(url: &str, timeout: Duration) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .connect_timeout(timeout)
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
    else {
        return false;
    };
    match client.head(url).send().await {
        Ok(r) => !r.status().is_server_error(),
        Err(e) => {
            tracing::info!("[reach] {url} unreachable: {e}");
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Region facts: which well-known services this machine cannot reach.
// ---------------------------------------------------------------------------

/// Services whose reachability a skill may need to know, by the name skills
/// read. A place is known by what it can reach, not by its locale or
/// timezone: a Mac set to Shanghai time on a VPN reaches everything.
const SERVICES: &[(&str, &str)] = &[
    ("youtube", "https://www.youtube.com/"),
    ("google", "https://www.google.com/"),
];

/// Re-probe after this long, so a VPN switched on or off is noticed.
const REGION_TTL: Duration = Duration::from_secs(10 * 60);

/// The general fact: the names of [`SERVICES`] this machine cannot reach
/// right now, and when that was checked (unix seconds).
#[derive(Debug, Clone, Default, serde::Serialize, PartialEq, Eq)]
pub struct Region {
    pub restricted: Vec<String>,
    pub checked_at: u64,
}

static REGION: std::sync::Mutex<Option<(std::time::Instant, Region)>> = std::sync::Mutex::new(None);
static PROBING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn fresh() -> Option<Region> {
    let guard = REGION.lock().ok()?;
    let (at, region) = guard.as_ref()?;
    (at.elapsed() < REGION_TTL).then(|| region.clone())
}

/// The last known fact, fresh or not, without probing — for callers that
/// must not wait (a skill tool's env).
pub fn region_cached() -> Option<Region> {
    REGION.lock().ok()?.as_ref().map(|(_, r)| r.clone())
}

fn restricted_of(results: &[(&str, bool)]) -> Vec<String> {
    results
        .iter()
        .filter(|(_, ok)| !ok)
        .map(|(name, _)| name.to_string())
        .collect()
}

/// The region fact, probing (every service at once, a few seconds at most)
/// when the cached one is missing or stale.
pub async fn region() -> Region {
    if let Some(r) = fresh() {
        return r;
    }
    let _one_probe = PROBING.lock().await;
    if let Some(r) = fresh() {
        return r;
    }
    let checks = SERVICES
        .iter()
        .map(|(name, url)| async move { (*name, reachable(url).await) });
    let results = futures_util::future::join_all(checks).await;
    let region = Region {
        restricted: restricted_of(&results),
        checked_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    };
    if !region.restricted.is_empty() {
        tracing::info!("[reach] unreachable here: {:?}", region.restricted);
    }
    if let Ok(mut g) = REGION.lock() {
        *g = Some((std::time::Instant::now(), region.clone()));
    }
    region
}

/// `GET /api/reach` — `{ restricted: ["youtube", …], checked_at }`.
pub async fn reach_api() -> axum::Json<Region> {
    axum::Json(region().await)
}

/// The fact as skill-tool env: `LINGGEN_RESTRICTED`, comma-separated
/// (empty = everything reachable). Absent until the first probe lands; a
/// stale value kicks off a re-probe for the next call.
pub fn tool_env() -> Option<(String, String)> {
    if fresh().is_none() {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(region());
        }
    }
    region_cached().map(|r| ("LINGGEN_RESTRICTED".to_string(), r.restricted.join(",")))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// Answers every connection with `status`. Returns its base URL.
    pub(crate) fn serve(status: u16) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let resp = format!(
                    "HTTP/1.1 {status} X\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    /// A URL nothing listens on — connection refused.
    pub(crate) fn dead() -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        drop(l);
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn an_answer_is_reachable_even_a_404() {
        assert!(reachable(&serve(200)).await);
        assert!(reachable(&serve(404)).await);
    }

    #[tokio::test]
    async fn refused_or_5xx_is_unreachable() {
        assert!(!reachable(&dead()).await);
        assert!(!reachable(&serve(503)).await);
    }

    #[test]
    fn restricted_names_the_unreachable_services_only() {
        assert_eq!(
            restricted_of(&[("youtube", false), ("google", false)]),
            vec!["youtube", "google"]
        );
        assert!(restricted_of(&[("youtube", true), ("google", true)]).is_empty());
        assert_eq!(
            restricted_of(&[("youtube", false), ("google", true)]),
            vec!["youtube"]
        );
    }

    #[test]
    fn the_region_serializes_as_skills_read_it() {
        let r = Region {
            restricted: vec!["youtube".into()],
            checked_at: 7,
        };
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            serde_json::json!({ "restricted": ["youtube"], "checked_at": 7 })
        );
    }

    #[tokio::test]
    async fn a_silent_host_times_out() {
        // Accepts the connection, never answers.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let _held: Vec<_> = listener.incoming().take(1).collect();
            std::thread::sleep(Duration::from_secs(5));
        });
        assert!(!reachable_within(&url, Duration::from_millis(300)).await);
    }
}

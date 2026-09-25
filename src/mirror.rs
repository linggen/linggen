//! linggen.dev `/dl` — the GitHub passthrough mirror for regions where
//! GitHub is blocked or flaky (notably mainland China). Every GitHub fetch
//! of our own repos goes GitHub first, mirror second.
//!
//! The mirror is a fallback, never a second opinion: a transport failure,
//! a timeout, a 5xx or a rate limit moves on to it; a 404 (or any other
//! answer) is GitHub's word and stands. Only the repos the mirror allowlists
//! map (linggensite `functions/api/_lib/dl.ts`) — anything else stays
//! GitHub-only. See linggensite doc/analytics-spec.md § China reachability.

use anyhow::Result;
use std::time::Duration;

const MIRROR: &str = "https://linggen.dev/dl";

/// Keep in step with `ALLOWED_REPOS` in linggensite's dl.ts.
const MIRRORED_REPOS: &[&str] = &[
    "linggen/linggen",
    "linggen/linggen-memory",
    "linggen/linggen-releases",
    "linggen/skills",
];

/// Connect timeout for clients whose fetches may fall back — short, so a
/// blackholed github.com costs seconds, not the default minutes.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(6);

/// The same resource on the mirror, or None when the URL is not one of our
/// mirrored GitHub shapes.
pub fn mirror_url(url: &str) -> Option<String> {
    let (base, query) = match url.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (url, None),
    };
    let path = base.strip_prefix("https://")?;
    let segs: Vec<&str> = path.split('/').collect();
    let inner = &segs[1..segs.len().saturating_sub(1)];
    if segs.iter().any(|s| *s == "." || *s == "..") || inner.iter().any(|s| s.is_empty()) {
        return None;
    }
    let mapped = match segs.as_slice() {
        ["github.com", o, r, "releases", "download", tag, asset] => {
            format!("release/{o}/{r}/{tag}/{asset}")
        }
        ["github.com", o, r, "releases", "latest", "download", asset] => {
            format!("latest/{o}/{r}/{asset}")
        }
        ["github.com", o, r, "archive", rest @ ..] if !rest.is_empty() => {
            let git_ref = rest.join("/");
            format!("zip/{o}/{r}/{}", git_ref.strip_suffix(".zip")?)
        }
        ["api.github.com", "repos", o, r, "releases"] => {
            return allowed(o, r).then(|| format!("{MIRROR}/api/{o}/{r}/releases"));
        }
        ["api.github.com", "repos", o, r, "releases", "latest"] => {
            format!("api/{o}/{r}/releases/latest")
        }
        ["api.github.com", "repos", o, r, "contents", rest @ ..] => {
            let tail = rest.join("/");
            let q = query
                .and_then(|q| q.split('&').find(|p| p.starts_with("ref=")))
                .map(|p| format!("?{p}"))
                .unwrap_or_default();
            format!("contents/{o}/{r}/{tail}{q}")
        }
        ["raw.githubusercontent.com", o, r, git_ref, rest @ ..] if !rest.is_empty() => {
            format!("raw/{o}/{r}/{git_ref}/{}", rest.join("/"))
        }
        _ => return None,
    };
    let mut parts = mapped.splitn(3, '/').skip(1);
    let (o, r) = (parts.next()?, parts.next()?.split('/').next()?);
    allowed(o, r).then(|| format!("{MIRROR}/{mapped}"))
}

fn allowed(owner: &str, repo: &str) -> bool {
    MIRRORED_REPOS.contains(&format!("{owner}/{repo}").as_str())
}

/// A source's definitive non-success answer (a 404, or the last source's
/// 5xx). Callers downcast to it to tell "doesn't exist" from "unreachable".
#[derive(Debug)]
pub struct HttpStatus {
    pub url: String,
    pub status: reqwest::StatusCode,
}

impl std::fmt::Display for HttpStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: HTTP {}", self.url, self.status)
    }
}

impl std::error::Error for HttpStatus {}

/// `url` then its mirror, when it has one.
pub fn sources(url: &str) -> Vec<String> {
    std::iter::once(url.to_string())
        .chain(mirror_url(url))
        .collect()
}

/// Whether this answer means "this source is unwell, try the next one"
/// rather than a real answer about the resource.
fn is_unwell(resp: &reqwest::Response) -> bool {
    let s = resp.status();
    let rate_limited = s == reqwest::StatusCode::FORBIDDEN
        && resp
            .headers()
            .get("x-ratelimit-remaining")
            .is_some_and(|v| v == "0");
    s.is_server_error() || s == reqwest::StatusCode::TOO_MANY_REQUESTS || rate_limited
}

/// GET `url` (GitHub first, then the mirror) and return the body. A 404 or
/// other definitive status from a source is returned as the error without
/// trying the mirror.
pub async fn get_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    get_bytes_from(client, &sources(url), &[]).await
}

/// [`get_bytes`] with extra request headers (e.g. a GitHub `Accept`).
pub async fn get_bytes_with(
    client: &reqwest::Client,
    url: &str,
    headers: &[(&str, &str)],
) -> Result<Vec<u8>> {
    get_bytes_from(client, &sources(url), headers).await
}

/// [`get_bytes`] decoded as JSON.
pub async fn get_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
) -> Result<T> {
    let body = get_bytes_with(client, url, &[("Accept", "application/vnd.github+json")]).await?;
    Ok(serde_json::from_slice(&body)?)
}

async fn get_bytes_from(
    client: &reqwest::Client,
    urls: &[String],
    headers: &[(&str, &str)],
) -> Result<Vec<u8>> {
    let mut last = None;
    for (i, url) in urls.iter().enumerate() {
        let has_next = i + 1 < urls.len();
        let mut req = client.get(url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("[mirror] {url} unreachable: {e}");
                last = Some(anyhow::anyhow!("{url}: {e}"));
                continue;
            }
        };
        if !resp.status().is_success() {
            let status = resp.status();
            if is_unwell(&resp) && has_next {
                tracing::warn!("[mirror] {url} answered {status}; trying the mirror");
                last = Some(anyhow::anyhow!("{url}: HTTP {status}"));
                continue;
            }
            return Err(HttpStatus {
                url: url.clone(),
                status,
            }
            .into());
        }
        match resp.bytes().await {
            Ok(b) => return Ok(b.to_vec()),
            Err(e) => {
                tracing::warn!("[mirror] {url} body failed: {e}");
                last = Some(anyhow::anyhow!("{url}: {e}"));
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no source to fetch")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// A one-shot-per-connection HTTP server answering every request with
    /// `status` and `body`. Returns its base URL.
    fn serve(status: u16, body: &'static str) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let resp = format!(
                    "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    /// A URL nothing listens on — connection refused.
    fn dead() -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        drop(l);
        format!("http://{addr}")
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn unreachable_primary_falls_back_to_mirror() {
        let urls = [dead(), serve(200, "from-mirror")];
        let got = get_bytes_from(&client(), &urls, &[]).await.unwrap();
        assert_eq!(&got[..], b"from-mirror");
    }

    #[tokio::test]
    async fn server_error_falls_back_to_mirror() {
        let urls = [serve(502, "bad"), serve(200, "from-mirror")];
        let got = get_bytes_from(&client(), &urls, &[]).await.unwrap();
        assert_eq!(&got[..], b"from-mirror");
    }

    #[tokio::test]
    async fn not_found_is_final() {
        let urls = [serve(404, "nope"), serve(200, "from-mirror")];
        let err = get_bytes_from(&client(), &urls, &[]).await.unwrap_err();
        let status = err.downcast_ref::<HttpStatus>().map(|h| h.status.as_u16());
        assert_eq!(status, Some(404), "{err}");
    }

    #[tokio::test]
    async fn healthy_primary_never_touches_mirror() {
        let urls = [serve(200, "from-github"), dead()];
        let got = get_bytes_from(&client(), &urls, &[]).await.unwrap();
        assert_eq!(&got[..], b"from-github");
    }

    #[tokio::test]
    async fn both_down_reports_the_last_failure() {
        let urls = [dead(), serve(503, "down")];
        let err = get_bytes_from(&client(), &urls, &[]).await.unwrap_err();
        assert!(err.to_string().contains("503"), "{err}");
    }

    #[test]
    fn maps_release_assets_and_latest() {
        assert_eq!(
            mirror_url("https://github.com/linggen/linggen/releases/download/1.8.2/ling-macos-aarch64.tar.gz")
                .as_deref(),
            Some("https://linggen.dev/dl/release/linggen/linggen/1.8.2/ling-macos-aarch64.tar.gz")
        );
        assert_eq!(
            mirror_url("https://github.com/linggen/linggen/releases/latest/download/manifest.json")
                .as_deref(),
            Some("https://linggen.dev/dl/latest/linggen/linggen/manifest.json")
        );
        assert_eq!(
            mirror_url(
                "https://api.github.com/repos/linggen/linggen-releases/releases?per_page=20"
            )
            .as_deref(),
            Some("https://linggen.dev/dl/api/linggen/linggen-releases/releases")
        );
    }

    #[test]
    fn maps_skills_contents_raw_and_zip() {
        assert_eq!(
            mirror_url("https://api.github.com/repos/linggen/skills/contents/").as_deref(),
            Some("https://linggen.dev/dl/contents/linggen/skills/")
        );
        assert_eq!(
            mirror_url("https://api.github.com/repos/linggen/skills/contents/dj?ref=main&x=1")
                .as_deref(),
            Some("https://linggen.dev/dl/contents/linggen/skills/dj?ref=main")
        );
        assert_eq!(
            mirror_url("https://raw.githubusercontent.com/linggen/skills/main/dj/SKILL.md")
                .as_deref(),
            Some("https://linggen.dev/dl/raw/linggen/skills/main/dj/SKILL.md")
        );
        assert_eq!(
            mirror_url("https://github.com/linggen/skills/archive/refs/heads/main.zip").as_deref(),
            Some("https://linggen.dev/dl/zip/linggen/skills/refs/heads/main")
        );
    }

    #[test]
    fn third_party_and_odd_urls_have_no_mirror() {
        for url in [
            "https://github.com/someone/skills/archive/refs/heads/main.zip",
            "https://raw.githubusercontent.com/someone/x/main/SKILL.md",
            "https://api.github.com/repos/linggen/linggen-app/releases",
            "https://clawhub.ai/api/v1/download?slug=x",
            "https://github.com/linggen/skills/archive/refs/heads/../main.zip",
            "http://github.com/linggen/linggen/releases/latest/download/manifest.json",
        ] {
            assert_eq!(mirror_url(url), None, "{url}");
        }
    }
}

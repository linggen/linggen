use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

const DEFAULT_MAX_BYTES: usize = 100 * 1024; // 100 KB

/// How much of an HTML page's markup is read, whatever `max_bytes` asks
/// of its text: scripts and styles are stripped after reading, and a page's
/// head alone can outweigh a small text limit many times over.
const HTML_READ_CEILING: usize = 2 * 1024 * 1024;

fn http_client() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            !(ip.is_private() || ip.is_loopback() || ip.is_link_local()
                || a == 0 || a >= 224 || (a == 100 && (64..128).contains(&b)))
        }
        IpAddr::V6(ip) => ip.to_ipv4_mapped().map_or_else(
            || ip.segments()[0] & 0xe000 == 0x2000,
            |ip| public_ip(ip.into()),
        ),
    }
}

async fn public_response(mut url: reqwest::Url) -> Result<reqwest::Response> {
    for hop in 0..=10 {
        let host = url.host().context("WebFetch needs an HTTP(S) host")?;
        let port = url.port_or_known_default().context("WebFetch needs an HTTP(S) URL")?;
        anyhow::ensure!(matches!(url.scheme(), "http" | "https"), "WebFetch needs an HTTP(S) URL");
        let (domain, addresses): (Option<&str>, Vec<SocketAddr>) = match host {
            url::Host::Domain(name) => (Some(name), tokio::net::lookup_host((name, port))
                .await.context("WebFetch could not resolve host")?.collect()),
            url::Host::Ipv4(ip) => (None, vec![SocketAddr::new(ip.into(), port)]),
            url::Host::Ipv6(ip) => (None, vec![SocketAddr::new(ip.into(), port)]),
        };
        anyhow::ensure!(!addresses.is_empty() && addresses.iter().all(|a| public_ip(a.ip())),
            "WebFetch only connects to public addresses; use Bash curl for a local URL.");
        let mut builder = http_client();
        if let Some(domain) = domain {
            // Use precisely the addresses checked above, not a second DNS lookup.
            builder = builder.resolve_to_addrs(domain, &addresses);
        }
        let response = builder.build()?.get(url.clone()).send().await.context("failed to fetch URL")?;
        if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            return Ok(response);
        }
        let Some(location) = response.headers().get(reqwest::header::LOCATION) else {
            return Ok(response);
        };
        anyhow::ensure!(hop < 10, "WebFetch redirect limit reached");
        url = url.join(location.to_str()?).context("invalid WebFetch redirect")?;
    }
    unreachable!()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WebFetchResult {
    pub url: String,
    pub content: String,
    pub content_type: String,
    pub truncated: bool,
    /// The page's title and pictures, for an HTML page.
    pub title: String,
    pub pictures: Vec<super::page_pictures::WebPicture>,
}

/// Fetch a URL and return its content as text.
///
/// - HTML responses are stripped of tags to produce plain text.
/// - Non-HTML (JSON, plain text, etc.) is returned as-is.
/// - A status other than success fails the fetch, naming the code.
/// - The body is read only up to the limit: `max_bytes` of a text body,
///   [`HTML_READ_CEILING`] of an HTML page's markup.
pub async fn fetch_url(url: &str, max_bytes: Option<usize>) -> Result<WebFetchResult> {
    let parsed = reqwest::Url::parse(url).context("invalid WebFetch URL")?;
    let resp = tokio::time::timeout(Duration::from_secs(15), public_response(parsed))
        .await.context("WebFetch timed out")??;
    read_response(url, max_bytes, resp).await
}

async fn read_response(url: &str, max_bytes: Option<usize>, resp: reqwest::Response) -> Result<WebFetchResult> {
    let limit = max_bytes.unwrap_or(DEFAULT_MAX_BYTES);
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {status} from {}", resp.url());
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("text/plain")
        .to_string();

    // Relative picture addresses resolve against where the redirects ended.
    let final_url = resp.url().to_string();
    let mut facts = super::page_pictures::PageFacts::default();
    let mut cut = false;
    let text = match body_kind(&content_type) {
        BodyKind::Html => {
            let (bytes, more) = read_head(resp, HTML_READ_CEILING).await?;
            cut = more;
            let html = utf8_head(&bytes);
            facts = super::page_pictures::read(&html, &final_url);
            strip_html_tags(&html)
        }
        BodyKind::Text => {
            let (bytes, more) = read_head(resp, limit).await?;
            cut = more;
            utf8_head(&bytes)
        }
        // Octet-stream is sometimes a plain file served without a type.
        BodyKind::Unknown => {
            let length = resp.content_length();
            let (bytes, more) = read_head(resp, limit).await?;
            cut = more;
            match std::str::from_utf8(&bytes) {
                Ok(_) => utf8_head(&bytes),
                Err(e) if more && e.error_len().is_none() => utf8_head(&bytes),
                Err(_) => binary_note(&content_type, length.or(Some(bytes.len() as u64))),
            }
        }
        BodyKind::Binary => binary_note(&content_type, resp.content_length()),
    };

    let truncated = cut || text.len() > limit;
    let content = if truncated {
        // Truncate at a char boundary
        let mut end = limit;
        while !text.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        text[..end].to_string()
    } else {
        text
    };

    Ok(WebFetchResult {
        url: url.to_string(),
        content,
        content_type,
        truncated,
        title: facts.title,
        pictures: facts.pictures,
    })
}

/// The body's first `cap` bytes, and whether there was more.
async fn read_head(mut resp: reqwest::Response, cap: usize) -> Result<(Vec<u8>, bool)> {
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.context("failed to read response body")? {
        let room = cap - body.len();
        if chunk.len() > room {
            body.extend_from_slice(&chunk[..room]);
            return Ok((body, true));
        }
        body.extend_from_slice(&chunk);
    }
    Ok((body, false))
}

/// Bytes as text: a character the cut split at the end is dropped, any
/// other invalid byte replaced.
fn utf8_head(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(e) if e.error_len().is_none() => {
            String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned()
        }
        Err(_) => String::from_utf8_lossy(bytes).into_owned(),
    }
}

#[derive(Debug, PartialEq)]
enum BodyKind {
    Html,
    Text,
    Unknown,
    Binary,
}

/// What a response body is, by its media type. A PDF or an image decoded as
/// UTF-8 is tens of KB of U+FFFD noise the model can't use — and that noise
/// once panicked the result preview and hung the turn.
fn body_kind(content_type: &str) -> BodyKind {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let binary_family = ["image/", "audio/", "video/", "font/"]
        .iter()
        .any(|p| mime.starts_with(p));
    match mime.as_str() {
        "text/html" | "application/xhtml+xml" => BodyKind::Html,
        "application/octet-stream" => BodyKind::Unknown,
        "application/pdf"
        | "application/zip"
        | "application/gzip"
        | "application/x-tar"
        | "application/msword"
        | "application/vnd.ms-excel"
        | "application/vnd.ms-powerpoint" => BodyKind::Binary,
        _ if binary_family
            || mime.starts_with("application/vnd.openxmlformats-officedocument.") =>
        {
            BodyKind::Binary
        }
        _ => BodyKind::Text,
    }
}

/// Said in place of a body that isn't text.
fn binary_note(content_type: &str, len: Option<u64>) -> String {
    let size = len.map(|n| format!(", {n} bytes")).unwrap_or_default();
    format!("[{content_type}{size} — not text. WebFetch reads web pages and text only.]")
}

/// Strip HTML tags and collapse whitespace to produce readable plain text.
fn strip_html_tags(html: &str) -> String {
    // Remove <script> and <style> blocks entirely
    let script_re = Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
    let style_re = Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    let text = script_re.replace_all(html, "");
    let text = style_re.replace_all(&text, "");

    // Strip remaining HTML tags
    let tag_re = Regex::new(r"<[^>]+>").unwrap();
    let text = tag_re.replace_all(&text, "");

    // Decode common HTML entities
    let text = text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");

    // Collapse runs of whitespace into single spaces, trim lines
    let ws_re = Regex::new(r"[ \t]+").unwrap();
    let blank_re = Regex::new(r"\n{3,}").unwrap();
    let text = ws_re.replace_all(&text, " ");
    let text = blank_re.replace_all(&text, "\n\n");

    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pdf_or_image_is_named_not_decoded() {
        for ct in [
            "application/pdf",
            "image/png",
            "application/zip",
            "font/woff2",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ] {
            assert_eq!(body_kind(ct), BodyKind::Binary, "{ct}");
        }
        assert_eq!(body_kind("text/html; charset=utf-8"), BodyKind::Html);
        assert_eq!(body_kind("application/json"), BodyKind::Text);
        assert_eq!(body_kind("application/vnd.api+json"), BodyKind::Text);
        assert_eq!(body_kind("text/plain"), BodyKind::Text);
        assert_eq!(body_kind("application/octet-stream"), BodyKind::Unknown);
        assert_eq!(
            binary_note("application/pdf", Some(237911)),
            "[application/pdf, 237911 bytes — not text. WebFetch reads web pages and text only.]"
        );
    }

    /// Serve one request on a local port: `head`, then `body` written again
    /// and again until the reader stops reading (or `repeat` times).
    fn serve_once(head: &'static str, body: &'static str, repeat: usize) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = conn.read(&mut request);
            let _ = conn.write_all(head.as_bytes());
            for _ in 0..repeat {
                if conn.write_all(body.as_bytes()).is_err() {
                    return;
                }
            }
        });
        format!("http://{addr}/page")
    }

    #[tokio::test]
    async fn fetch_refuses_local_destinations() {
        let url = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n",
            "test",
            1,
        );
        let err = fetch_url(&url, None).await.expect_err("WebFetch is for public addresses");
        assert!(format!("{err:#}").contains("public"), "{err:#}");
    }

    async fn read_fixture(url: &str, max_bytes: Option<usize>) -> Result<WebFetchResult> {
        let response = http_client().build()?.get(url).send().await?;
        read_response(url, max_bytes, response).await
    }

    #[tokio::test]
    async fn a_failed_status_is_told_with_its_code() {
        let url = serve_once(
            "HTTP/1.1 404 Not Found\r\nContent-Type: text/html\r\nContent-Length: 22\r\nConnection: close\r\n\r\n",
            "<p>no such page</p>\n\n\n",
            1,
        );
        let err = read_fixture(&url, None).await.expect_err("a 404 is no page");
        assert!(format!("{err:#}").contains("404"), "{err:#}");
    }

    #[tokio::test]
    async fn the_body_is_read_only_up_to_the_limit() {
        let url = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            usize::MAX,
        );
        let started = std::time::Instant::now();
        let page = read_fixture(&url, Some(1000)).await.expect("the head of the body");
        assert!(started.elapsed() < Duration::from_secs(5), "read on: {:?}", started.elapsed());
        assert!(page.truncated);
        assert_eq!(page.content.len(), 1000);
    }

    /// A small `max_bytes` still reads past a page's heavy head: the limit
    /// is on the text, the markup has a ceiling of its own.
    #[tokio::test]
    async fn a_heavy_head_does_not_hide_the_page_text() {
        let script = "<script>var x = 1;</script>".repeat(2000);
        let page: &'static str = Box::leak(
            format!("<html><head><title>T</title>{script}</head><body><p>THE PAGE TEXT</p></body></html>")
                .into_boxed_str(),
        );
        let url = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n",
            page,
            1,
        );
        let fetched = read_fixture(&url, Some(1000)).await.unwrap();
        assert!(fetched.content.contains("THE PAGE TEXT"), "{}", fetched.content);
        assert!(!fetched.content.contains("var x"), "{}", fetched.content);
    }

    #[test]
    fn test_strip_html_basic() {
        let html = "<p>Hello <b>world</b></p>";
        assert_eq!(strip_html_tags(html), "Hello world");
    }

    #[test]
    fn test_strip_html_script_and_style() {
        let html = r#"
            <html>
            <head><style>body { color: red; }</style></head>
            <body>
            <script>alert('hi');</script>
            <p>Content here</p>
            </body>
            </html>
        "#;
        let text = strip_html_tags(html);
        assert!(!text.contains("color: red"));
        assert!(!text.contains("alert"));
        assert!(text.contains("Content here"));
    }

    #[test]
    fn test_strip_html_entities() {
        let html = "<p>A &amp; B &lt; C &gt; D &quot;E&quot; F&#39;s</p>";
        assert_eq!(strip_html_tags(html), r#"A & B < C > D "E" F's"#);
    }

    #[test]
    fn test_strip_html_whitespace_collapse() {
        let html = "<p>  lots   of   spaces  </p>\n\n\n\n\n<p>next</p>";
        let text = strip_html_tags(html);
        assert!(!text.contains("   "));
        assert!(text.contains("lots of spaces"));
        assert!(text.contains("next"));
    }

    #[test]
    fn test_truncation() {
        let long = "a".repeat(200);
        let result = tokio::runtime::Runtime::new().unwrap().block_on(async {
            // We can't actually fetch a URL in unit tests, so test truncation logic directly
            let limit = 100usize;
            let truncated = long.len() > limit;
            let content = if truncated {
                long[..limit].to_string()
            } else {
                long.clone()
            };
            (content, truncated)
        });
        assert_eq!(result.0.len(), 100);
        assert!(result.1);
    }

    #[test]
    fn test_truncation_char_boundary() {
        // Multi-byte chars: each is 3 bytes in UTF-8
        let text = "aaaa\u{00e9}\u{00e9}"; // 4 + 2*2 = 8 chars, but 4 + 2*2 = 8 bytes (é is 2 bytes)
                                           // With a limit that might land in the middle of a multi-byte char
        let limit = 5;
        let mut end = limit;
        while !text.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        let truncated = &text[..end];
        // Should not panic and should be valid UTF-8
        assert!(truncated.len() <= limit);
        assert!(truncated.is_char_boundary(truncated.len()));
    }

    #[test]
    fn test_strip_html_non_html() {
        // Plain text passed through should come back mostly unchanged
        let plain = "Just some plain text content.";
        assert_eq!(strip_html_tags(plain), "Just some plain text content.");
    }
}

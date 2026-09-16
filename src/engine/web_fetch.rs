use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use std::time::Duration;

const DEFAULT_MAX_BYTES: usize = 100 * 1024; // 100 KB

static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(10))
        .pool_max_idle_per_host(2)
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()
        .expect("failed to build shared HTTP client")
});

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WebFetchResult {
    pub url: String,
    pub content: String,
    pub content_type: String,
    pub truncated: bool,
}

/// Fetch a URL and return its content as text.
///
/// - HTML responses are stripped of tags to produce plain text.
/// - Non-HTML (JSON, plain text, etc.) is returned as-is.
/// - Content is truncated to `max_bytes` to avoid blowing up context.
pub async fn fetch_url(url: &str, max_bytes: Option<usize>) -> Result<WebFetchResult> {
    let limit = max_bytes.unwrap_or(DEFAULT_MAX_BYTES);

    let resp = HTTP_CLIENT
        .get(url)
        .send()
        .await
        .context("failed to fetch URL")?;

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("text/plain")
        .to_string();

    let text = match body_kind(&content_type) {
        BodyKind::Html => {
            strip_html_tags(&resp.text().await.context("failed to read response body")?)
        }
        BodyKind::Text => resp.text().await.context("failed to read response body")?,
        // Octet-stream is sometimes a plain file served without a type.
        BodyKind::Unknown => {
            let bytes = resp.bytes().await.context("failed to read response body")?;
            match String::from_utf8(bytes.to_vec()) {
                Ok(text) => text,
                Err(_) => binary_note(&content_type, Some(bytes.len() as u64)),
            }
        }
        BodyKind::Binary => binary_note(&content_type, resp.content_length()),
    };

    let truncated = text.len() > limit;
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
    })
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

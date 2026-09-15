use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use std::time::Duration;

static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .pool_max_idle_per_host(2)
        .build()
        .expect("failed to build shared HTTP client")
});

const TAVILY_URL: &str = "https://api.tavily.com/search";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Search the web. A Tavily key saved in Settings goes straight to Tavily:
/// the user chose it, it costs nothing against the account's pool, and it
/// keeps working when a plan lapses. With no key, search goes through the
/// Linggen Cloud proxy (Tavily behind the account token), which meters each
/// search against the account's monthly pool, so sign-in is required.
///
/// A failing key is reported, never silently swapped for the cloud — a quiet
/// fallback would hide that the key the user saved is broken.
pub async fn web_search(query: &str, max_results: usize) -> Result<Vec<WebSearchResult>> {
    if let Some(key) = own_tavily_key() {
        return tavily_search(&key, query, max_results).await;
    }
    // The AUTH_REQUIRED prefix is what routes this to the chat UI's inline
    // sign-in button instead of a bare tool_error line.
    let (token, _) = crate::account::resolve_token()
        .context("AUTH_REQUIRED: Please sign in to linggen.dev to use web search.")?;

    cloud_search(&token, query, max_results).await
}

/// The Tavily key from Settings, read fresh so a key saved mid-session is
/// used on the next search without a restart.
fn own_tavily_key() -> Option<String> {
    let creds = crate::credentials::Credentials::load(&crate::credentials::credentials_file());
    creds.service_key("tavily").map(str::to_string)
}

/// Search response — `{ results: [{ title, url, content }] }`. Tavily answers
/// in this shape, and the cloud proxy (`linggensite/functions/api/_lib/search.ts`)
/// passes it through.
#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    results: Vec<SearchItem>,
}

#[derive(Debug, Deserialize)]
struct SearchItem {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    content: String,
}

async fn tavily_search(key: &str, query: &str, max_results: usize) -> Result<Vec<WebSearchResult>> {
    let body = serde_json::json!({
        "query": query,
        "max_results": max_results,
        "include_answer": false,
    });

    let resp = HTTP_CLIENT
        .post(TAVILY_URL)
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .context("failed to reach Tavily")?;

    let status = resp.status();
    if !status.is_success() {
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        anyhow::bail!(
            "Tavily refused the key saved in Settings: {} ({})",
            tavily_error(&body),
            status
        );
    }
    read_results(resp, max_results).await
}

/// Tavily's error text. It answers `{"detail": {"error": "..."}}`, with a
/// bare `detail` string on some errors.
fn tavily_error(body: &serde_json::Value) -> &str {
    let detail = body.get("detail");
    detail
        .and_then(|d| d.get("error"))
        .and_then(|e| e.as_str())
        .or_else(|| detail.and_then(|d| d.as_str()))
        .unwrap_or("no reason given")
}

async fn cloud_search(
    token: &str,
    query: &str,
    max_results: usize,
) -> Result<Vec<WebSearchResult>> {
    let url = format!("{}/api/search", crate::account::site_url());
    let body = serde_json::json!({
        "query": query,
        "max_results": max_results,
    });

    let resp = HTTP_CLIENT
        .post(&url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .context("failed to reach the Linggen search service")?;

    let status = resp.status();
    if !status.is_success() {
        // An expired or rejected token reads to the user as "not signed in".
        if status == reqwest::StatusCode::UNAUTHORIZED {
            anyhow::bail!("AUTH_REQUIRED: Please sign in to linggen.dev to use web search.");
        }
        // Otherwise surface the server's own message (trial used up, monthly
        // cap reached, not configured, …) verbatim.
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        let msg = body
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("web search failed");
        anyhow::bail!("{} ({})", msg, status);
    }
    read_results(resp, max_results).await
}

async fn read_results(resp: reqwest::Response, max_results: usize) -> Result<Vec<WebSearchResult>> {
    let parsed: SearchResponse = resp
        .json()
        .await
        .context("failed to parse the search response")?;

    let results = parsed
        .results
        .into_iter()
        .take(max_results)
        .map(|r| WebSearchResult {
            title: r.title,
            url: r.url,
            snippet: r.content,
        })
        .collect();

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_search_response() {
        let json = r#"{
            "results": [
                {"title": "Example", "url": "https://example.com", "content": "A snippet here", "score": 0.9},
                {"title": "Test Page", "url": "https://test.com", "content": "Another snippet", "score": 0.8}
            ]
        }"#;
        let resp: SearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.results.len(), 2);
        assert_eq!(resp.results[0].title, "Example");
        assert_eq!(resp.results[0].url, "https://example.com");
        assert_eq!(resp.results[0].content, "A snippet here");
    }

    #[test]
    fn test_parse_search_response_empty() {
        let json = r#"{"results": []}"#;
        let resp: SearchResponse = serde_json::from_str(json).unwrap();
        assert!(resp.results.is_empty());
    }

    #[test]
    fn test_tavily_error_shapes() {
        let nested =
            serde_json::json!({"detail": {"error": "Unauthorized: missing or invalid API key."}});
        assert_eq!(
            tavily_error(&nested),
            "Unauthorized: missing or invalid API key."
        );
        let flat = serde_json::json!({"detail": "Plan limit exceeded"});
        assert_eq!(tavily_error(&flat), "Plan limit exceeded");
        assert_eq!(tavily_error(&serde_json::Value::Null), "no reason given");
    }

    #[test]
    fn test_search_item_to_web_search_result() {
        let item = SearchItem {
            title: "Rust Lang".to_string(),
            url: "https://rust-lang.org".to_string(),
            content: "A systems programming language".to_string(),
        };
        let result = WebSearchResult {
            title: item.title,
            url: item.url,
            snippet: item.content,
        };
        assert_eq!(result.title, "Rust Lang");
        assert_eq!(result.snippet, "A systems programming language");
    }
}

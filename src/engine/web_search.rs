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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Search the web through the Linggen Cloud proxy (Tavily behind the account
/// token). There is no per-user Tavily key: the proxy holds the key and meters
/// each search against the account's monthly pool, so sign-in is required.
pub async fn web_search(query: &str, max_results: usize) -> Result<Vec<WebSearchResult>> {
    let (token, _) = crate::account::resolve_token()
        .context("Please sign in to linggen.dev to use web search.")?;

    cloud_search(&token, query, max_results).await
}

/// Cloud search proxy response — `{ results: [{ title, url, content }] }`,
/// matching `linggensite/functions/api/_lib/search.ts`.
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
            anyhow::bail!("Please sign in to linggen.dev to use web search.");
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

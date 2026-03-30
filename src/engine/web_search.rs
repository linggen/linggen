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

/// Search the web using Tavily API and return structured results.
///
/// Requires a Tavily API key stored in `~/.linggen/credentials.json`
/// under the key `"tavily"`, or via env var `TAVILY_API_KEY`.
pub async fn web_search(query: &str, max_results: usize) -> Result<Vec<WebSearchResult>> {
    let api_key = resolve_tavily_key()
        .context("Tavily API key not found. Get a free key at https://tavily.com then set it in ~/.linggen/credentials.json under \"tavily\" or env var TAVILY_API_KEY")?;

    tavily_search(&api_key, query, max_results).await
}

/// Resolve the Tavily API key from credentials.json or env var.
fn resolve_tavily_key() -> Option<String> {
    // 1. credentials.json
    let creds = crate::credentials::Credentials::load(&crate::credentials::credentials_file());
    if let Some(key) = creds.get_api_key("tavily") {
        if !key.is_empty() {
            return Some(key.to_string());
        }
    }
    // 2. Environment variable
    if let Ok(key) = std::env::var("TAVILY_API_KEY") {
        if !key.is_empty() {
            return Some(key);
        }
    }
    None
}

/// Tavily search API response.
#[derive(Debug, Deserialize)]
struct TavilyResponse {
    results: Vec<TavilyResult>,
}

#[derive(Debug, Deserialize)]
struct TavilyResult {
    title: String,
    url: String,
    content: String,
}

async fn tavily_search(
    api_key: &str,
    query: &str,
    max_results: usize,
) -> Result<Vec<WebSearchResult>> {
    let body = serde_json::json!({
        "query": query,
        "max_results": max_results,
        "include_answer": false,
    });

    let resp = HTTP_CLIENT
        .post("https://api.tavily.com/search")
        .header("Content-Type", "application/json")
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .context("failed to call Tavily search API")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Tavily API error ({}): {}", status, text);
    }

    let tavily_resp: TavilyResponse = resp
        .json()
        .await
        .context("failed to parse Tavily response")?;

    let results = tavily_resp
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
    fn test_parse_tavily_response() {
        let json = r#"{
            "results": [
                {"title": "Example", "url": "https://example.com", "content": "A snippet here", "score": 0.9},
                {"title": "Test Page", "url": "https://test.com", "content": "Another snippet", "score": 0.8}
            ]
        }"#;
        let resp: TavilyResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.results.len(), 2);
        assert_eq!(resp.results[0].title, "Example");
        assert_eq!(resp.results[0].url, "https://example.com");
        assert_eq!(resp.results[0].content, "A snippet here");
    }

    #[test]
    fn test_parse_tavily_response_empty() {
        let json = r#"{"results": []}"#;
        let resp: TavilyResponse = serde_json::from_str(json).unwrap();
        assert!(resp.results.is_empty());
    }

    #[test]
    fn test_tavily_to_web_search_result() {
        let tavily = TavilyResult {
            title: "Rust Lang".to_string(),
            url: "https://rust-lang.org".to_string(),
            content: "A systems programming language".to_string(),
        };
        let result = WebSearchResult {
            title: tavily.title,
            url: tavily.url,
            snippet: tavily.content,
        };
        assert_eq!(result.title, "Rust Lang");
        assert_eq!(result.snippet, "A systems programming language");
    }
}

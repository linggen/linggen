//! Model ids a key provider lists for a key: suggestions for the model input,
//! nothing more. Asked when a provider row opens (the page keeps them for its
//! life); nothing is hidden, ranked or priced. A list that cannot be read
//! gives no suggestions, and a full id typed + Enter still adds the model.
//! Nothing here logs, stores or returns a key.

use serde::Serialize;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(10);
/// A list is a few hundred ids; a page that is much bigger is not one.
const MAX_PAGES: usize = 20;
const MAX_IDS: usize = 5000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Suggestion {
    pub id: String,
    /// The provider's own name for it (Anthropic's `display_name`), when it gives one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Which list shape a provider answers in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wire {
    /// `GET {base}/models`, bearer key, `{"data":[{"id"}]}`.
    OpenAi,
    /// `GET {base}/v1/models`, `x-api-key`, paged, `display_name`.
    Anthropic,
    /// `GET {base}/api/tags`, `{"models":[{"name"}]}`.
    Ollama,
}

/// Why a list could not be read: the provider answered 401/403 to the key, or anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unread {
    KeyRejected,
    Unreachable,
}

impl Unread {
    /// The word `GET /api/providers/{id}/suggestions` tells a page.
    pub fn as_str(self) -> &'static str {
        match self {
            Unread::KeyRejected => "key_rejected",
            Unread::Unreachable => "unreachable",
        }
    }
}

/// The ids `base` lists for `key`, or why the list cannot be read.
pub async fn list(wire: Wire, base: &str, key: Option<&str>) -> Result<Vec<Suggestion>, Unread> {
    read_all(wire, base, key).await.ok_or(Unread::Unreachable).and_then(|r| r)
}

/// `None`: unreachable or unreadable; `Some(Err(KeyRejected))`: the provider refused the key.
async fn read_all(wire: Wire, base: &str, key: Option<&str>) -> Option<Result<Vec<Suggestion>, Unread>> {
    // No redirect: a key header must not follow a provider's answer elsewhere.
    let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(TIMEOUT).build().ok()?;
    let base = base.trim_end_matches('/');
    let mut out = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let request = match wire {
            Wire::Anthropic => {
                let mut url = format!("{base}/v1/models?limit=1000");
                if let Some(cursor) = &after {
                    url.push_str(&format!("&after_id={}", urlencoding::encode(cursor)));
                }
                client.get(url).header("x-api-key", key?).header("anthropic-version", "2023-06-01")
            }
            Wire::OpenAi => client.get(format!("{base}/models")).bearer_auth(key?),
            Wire::Ollama => client.get(format!("{base}/api/tags")),
        };
        let response = request.send().await.ok()?;
        if matches!(response.status().as_u16(), 401 | 403) {
            return Some(Err(Unread::KeyRejected));
        }
        if !response.status().is_success() {
            return None;
        }
        let body: serde_json::Value = response.json().await.ok()?;
        let (page, next) = read_page(&body, wire)?;
        out.extend(page);
        if out.len() > MAX_IDS {
            return None;
        }
        match next {
            Some(cursor) => after = Some(cursor),
            None => return Some(Ok(out)),
        }
    }
    None
}

/// One page's ids, and the next page's cursor. Gemini names models `models/<id>`.
fn read_page(body: &serde_json::Value, wire: Wire) -> Option<(Vec<Suggestion>, Option<String>)> {
    if wire == Wire::Ollama {
        let ids = body.get("models")?.as_array()?.iter()
            .filter_map(|m| m.get("name")?.as_str())
            .map(|id| Suggestion { id: id.to_string(), name: None })
            .collect();
        return Some((ids, None));
    }
    let ids = body.get("data")?.as_array()?.iter().filter_map(|item| {
        let id = item.get("id")?.as_str()?;
        let name = item.get("display_name").and_then(|n| n.as_str()).filter(|n| !n.trim().is_empty());
        Some(Suggestion { id: id.strip_prefix("models/").unwrap_or(id).to_string(), name: name.map(str::to_string) })
    }).collect();
    let more = wire == Wire::Anthropic && body.get("has_more").and_then(|v| v.as_bool()).unwrap_or(false);
    let next = more.then(|| body.get("last_id")?.as_str().map(str::to_string)).flatten();
    Some((ids, next))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn each_wire_is_read_in_its_own_shape() {
        let openai = json!({"data": [{"id": "gpt-x"}, {"id": "models/gemini-y"}]});
        let (ids, next) = read_page(&openai, Wire::OpenAi).unwrap();
        assert_eq!(ids.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["gpt-x", "gemini-y"]);
        assert!(next.is_none());
        let claude = json!({"data": [{"id": "claude-a", "display_name": "Claude A"}], "has_more": true, "last_id": "claude-a"});
        let (ids, next) = read_page(&claude, Wire::Anthropic).unwrap();
        assert_eq!(ids[0].name.as_deref(), Some("Claude A"));
        assert_eq!(next.as_deref(), Some("claude-a"));
        let ollama = json!({"models": [{"name": "qwen3:32b"}]});
        assert_eq!(read_page(&ollama, Wire::Ollama).unwrap().0[0].id, "qwen3:32b");
        assert!(read_page(&json!({"error": "no"}), Wire::OpenAi).is_none());
    }

    /// Nothing is hidden: a tts or embedding id is suggested like any other.
    #[test]
    fn no_id_is_hidden() {
        let body = json!({"data": [{"id": "tts-1"}, {"id": "text-embedding-3"}, {"id": "gpt-x"}]});
        assert_eq!(read_page(&body, Wire::OpenAi).unwrap().0.len(), 3);
    }
}

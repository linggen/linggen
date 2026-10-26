//! Video search: videos about a thing, with title, channel, duration and a
//! thumbnail (`doc/chat-spec.md` § Sources and pictures in answers).
//!
//! One web search on the same own-key / Linggen Cloud path as `WebSearch`,
//! narrowed to YouTube by the query itself (`site:` — the cloud proxy
//! passes only the query on). Each video's channel comes from YouTube's
//! public oEmbed, its duration from its watch page; both are best-effort
//! within a short budget, and a video is kept without them. The thumbnail
//! is YouTube's own still for the video id; like any picture it is only
//! downloaded when an answer shows the video.

use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use std::time::Duration;

/// Search results asked for; at most [`MAX_VIDEOS`] are kept.
const SEARCHED: usize = 8;
const MAX_VIDEOS: usize = 5;
/// The details' share of the search: past it a video goes without them.
const DETAILS_BUDGET: Duration = Duration::from_secs(4);
/// A watch page is read up to this far for its duration.
const WATCH_PAGE_LIMIT: usize = 2 * 1024 * 1024;

/// `LINGGEN_YOUTUBE_URL` points oEmbed, watch pages and thumbnails at a
/// stand-in (the system tests' loopback site).
fn youtube() -> String {
    std::env::var("LINGGEN_YOUTUBE_URL").unwrap_or_else(|_| "https://www.youtube.com".into())
}

fn thumbnail_url(id: &str) -> String {
    match std::env::var("LINGGEN_YOUTUBE_URL") {
        Ok(base) => format!("{base}/vi/{id}/hqdefault.jpg"),
        Err(_) => format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"),
    }
}

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(DETAILS_BUDGET)
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .build()
        .expect("failed to build the video client")
});

/// A video found for a search.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct FoundVideo {
    /// Its watch page.
    pub url: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub channel: String,
    /// Length in seconds, when its page said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<u32>,
    pub thumbnail: String,
    /// The address that plays it inside a page.
    pub embed: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub snippet: String,
    /// Its number in the session (`turn2video1`), once numbered.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
}

/// Videos for `query`, best first.
pub async fn video_search(query: &str) -> Result<Vec<FoundVideo>> {
    let narrowed = format!("site:youtube.com {query}");
    let results = super::web_search::web_search(&narrowed, SEARCHED).await?;
    let mut videos: Vec<FoundVideo> = Vec::new();
    for r in results {
        let Some(id) = youtube_id(&r.url) else {
            continue;
        };
        if videos.len() >= MAX_VIDEOS || videos.iter().any(|v| v.embed.ends_with(&id)) {
            continue;
        }
        videos.push(video(&id, &r.title, &r.snippet));
    }
    futures_util::future::join_all(videos.iter_mut().map(add_details)).await;
    Ok(videos)
}

fn video(id: &str, title: &str, snippet: &str) -> FoundVideo {
    FoundVideo {
        url: format!("https://www.youtube.com/watch?v={id}"),
        title: title.trim().trim_end_matches(" - YouTube").to_string(),
        thumbnail: thumbnail_url(id),
        embed: format!("https://www.youtube-nocookie.com/embed/{id}"),
        snippet: short(snippet, 200),
        ..Default::default()
    }
}

/// Whether a search result is a video page rather than one with pictures.
pub fn is_video_page(url: &str) -> bool {
    youtube_id(url).is_some()
}

/// The video id of a YouTube watch, short-link, shorts or embed address.
pub fn youtube_id(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed
        .host_str()?
        .trim_start_matches("www.")
        .trim_start_matches("m.");
    let id = match host {
        "youtube.com" | "youtube-nocookie.com" => youtube_path_id(&parsed)?,
        "youtu.be" => parsed.path_segments()?.next()?.to_string(),
        _ => return None,
    };
    valid_id(&id).then_some(id)
}

fn youtube_path_id(parsed: &url::Url) -> Option<String> {
    let mut segments = parsed.path_segments()?;
    match segments.next()? {
        "watch" => parsed
            .query_pairs()
            .find(|(k, _)| k == "v")
            .map(|(_, v)| v.into_owned()),
        "shorts" | "embed" | "live" => segments.next().map(str::to_string),
        _ => None,
    }
}

fn valid_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn short(text: &str, max_chars: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(max_chars) {
        Some((cut, _)) => format!("{}…", &flat[..cut]),
        None => flat,
    }
}

/// Its channel and duration, each within the budget, each optional.
async fn add_details(v: &mut FoundVideo) {
    let id = v.embed.rsplit('/').next().unwrap_or_default().to_string();
    let (channel, seconds) = tokio::join!(channel_of(&v.url), seconds_of(&id));
    v.channel = channel.unwrap_or_default();
    v.seconds = seconds;
}

#[derive(Deserialize)]
struct OEmbed {
    #[serde(default)]
    author_name: String,
}

async fn channel_of(watch_url: &str) -> Option<String> {
    let url = format!("{}/oembed", youtube());
    let resp = HTTP
        .get(url)
        .query(&[("format", "json"), ("url", watch_url)])
        .send()
        .await
        .ok()?;
    let found: OEmbed = resp.error_for_status().ok()?.json().await.ok()?;
    (!found.author_name.is_empty()).then_some(found.author_name)
}

static LENGTH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""lengthSeconds":"(\d{1,6})""#).unwrap());

/// Reads the watch page only until its length shows up.
async fn seconds_of(id: &str) -> Option<u32> {
    let url = format!("{}/watch?v={id}", youtube());
    let mut resp = HTTP
        .get(url)
        .header("Accept-Language", "en")
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    let mut page: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.ok()? {
        page.extend_from_slice(&chunk);
        if let Some(seconds) = length_in(&page) {
            return Some(seconds);
        }
        if page.len() > WATCH_PAGE_LIMIT {
            return None;
        }
    }
    None
}

fn length_in(page: &[u8]) -> Option<u32> {
    let text = String::from_utf8_lossy(page);
    LENGTH.captures(&text)?.get(1)?.as_str().parse().ok()
}

/// `4:05`, `1:02:03`.
pub fn clock(seconds: u32) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 {
        return format!("{h}:{m:02}:{s:02}");
    }
    format!("{m}:{s:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_ids_come_from_every_youtube_address_shape() {
        let id = Some("jv_ZAQrKla8".to_string());
        for url in [
            "https://www.youtube.com/watch?v=jv_ZAQrKla8",
            "https://m.youtube.com/watch?feature=share&v=jv_ZAQrKla8&t=30",
            "https://youtu.be/jv_ZAQrKla8?si=x",
            "https://www.youtube.com/shorts/jv_ZAQrKla8",
            "https://www.youtube-nocookie.com/embed/jv_ZAQrKla8",
        ] {
            assert_eq!(youtube_id(url), id, "{url}");
        }
        for url in [
            "https://www.youtube.com/@BridgeCoffee",
            "https://www.youtube.com/watch?v=short",
            "https://www.bilibili.com/video/BV1SJ4m1E771",
            "https://notyoutube.com/watch?v=jv_ZAQrKla8",
            "not a url",
        ] {
            assert_eq!(youtube_id(url), None, "{url}");
        }
    }

    #[test]
    fn a_video_plays_without_cookies_and_reads_clean() {
        let v = video(
            "jv_ZAQrKla8",
            " How to Dial In - YouTube ",
            "line one\n\n line   two",
        );
        assert_eq!(v.url, "https://www.youtube.com/watch?v=jv_ZAQrKla8");
        assert_eq!(
            v.embed,
            "https://www.youtube-nocookie.com/embed/jv_ZAQrKla8"
        );
        assert_eq!(v.title, "How to Dial In");
        assert_eq!(v.snippet, "line one line two");
        assert_eq!(short("一二三四五", 3), "一二三…");
    }

    #[test]
    fn a_watch_page_gives_its_length() {
        let page = br#"..."videoId":"x","lengthSeconds":"137","keywords"..."#;
        assert_eq!(length_in(page), Some(137));
        assert_eq!(length_in(b"no length here"), None);
        assert_eq!(clock(137), "2:17");
        assert_eq!(clock(59), "0:59");
        assert_eq!(clock(3723), "1:02:03");
    }
}

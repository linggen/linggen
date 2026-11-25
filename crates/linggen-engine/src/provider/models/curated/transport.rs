//! How the list is fetched: one GET with an ETag, bytes capped, no redirects
//! (the client is built in `store`). A body is read through `bundle::accept`
//! like every other copy.

pub(super) const MAX_ETAG: usize = 512;
/// The served list changes rarely; asking more often finds nothing new.
pub(super) const CHECK_EVERY_DAYS: i64 = 7;

impl Transport for reqwest::Client {
    async fn get(&self, url: &str, etag: Option<&str>, limit: usize) -> anyhow::Result<Reply> {
        let mut request = reqwest::Client::get(self, url);
        if let Some(tag) = etag { request = request.header(reqwest::header::IF_NONE_MATCH, tag); }
        let mut response = request.send().await?;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED { return Ok(Reply::Unchanged); }
        anyhow::ensure!(response.status() == reqwest::StatusCode::OK, "catalog not available");
        let etag = response.headers().get(reqwest::header::ETAG)
            .and_then(|tag| tag.to_str().ok()).map(str::to_string);
        anyhow::ensure!(response.content_length().is_none_or(|n| n <= limit as u64), "oversized catalog");
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            anyhow::ensure!(chunk.len() <= limit - bytes.len(), "oversized catalog");
            bytes.extend_from_slice(&chunk);
        }
        Ok(Reply::Body { bytes, etag })
    }
}

pub(super) fn valid_etag(tag: &str) -> bool {
    let quoted = tag.strip_prefix("W/").unwrap_or(tag);
    tag.len() <= MAX_ETAG && quoted.strip_prefix('"').and_then(|s| s.strip_suffix('"'))
        .is_some_and(|s| s.bytes().all(|b| b == 0x21 || (0x23..=0x7e).contains(&b)))
}

pub(super) enum Reply {
    Unchanged,
    Body { bytes: Vec<u8>, etag: Option<String> },
}

/// No credentials or configurable destination. A transport must cap the
/// streamed bytes at `limit` and report non-200/304 responses as errors.
pub(super) trait Transport {
    async fn get(&self, url: &str, etag: Option<&str>, limit: usize) -> anyhow::Result<Reply>;
}


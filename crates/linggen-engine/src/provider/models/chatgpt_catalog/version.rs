//! The Codex client version to present, read from the public release
//! listing: npm's `latest` for `@openai/codex`, else GitHub's latest release.
//! A plain GET with no token and nothing about the person. Only a plain
//! released version is taken (three numbers, no pre-release), and what is
//! presented never goes below the compiled value ([`Snapshot::client_version`]).

use super::{parse_version, Snapshot};
use async_trait::async_trait;

/// Where the latest published version comes from. Tests pass their own.
#[async_trait]
pub trait PublicListing: Send + Sync {
    /// The latest published version string, as the listing gives it.
    async fn latest(&self) -> anyhow::Result<String>;
}

const NPM_LATEST: &str = "https://registry.npmjs.org/@openai/codex/latest";
const GITHUB_LATEST: &str = "https://api.github.com/repos/openai/codex/releases/latest";

/// The real listing: npm first, GitHub when npm fails.
pub struct CodexReleases {
    http: reqwest::Client,
}

impl CodexReleases {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent("linggen")
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self { http }
    }

    async fn get(&self, url: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self.http.get(url).send().await?.error_for_status()?;
        Ok(resp.json().await?)
    }
}

impl Default for CodexReleases {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PublicListing for CodexReleases {
    async fn latest(&self) -> anyhow::Result<String> {
        if let Some(v) = self.get(NPM_LATEST).await.ok().as_ref().and_then(npm_version) {
            return Ok(v);
        }
        let json = self.get(GITHUB_LATEST).await?;
        github_version(&json).ok_or_else(|| anyhow::anyhow!("no released version on GitHub"))
    }
}

/// npm's answer: `{"version": "0.169.1", …}`.
pub fn npm_version(json: &serde_json::Value) -> Option<String> {
    let v = json.get("version")?.as_str()?;
    parse_version(v).map(|_| v.to_string())
}

/// GitHub's answer: `tag_name: "rust-v0.169.1"`, not a pre-release.
pub fn github_version(json: &serde_json::Value) -> Option<String> {
    if json.get("prerelease").and_then(|v| v.as_bool()) != Some(false) {
        return None;
    }
    let tag = json.get("tag_name")?.as_str()?;
    let v = tag.strip_prefix("rust-v").unwrap_or(tag);
    parse_version(v).map(|_| v.to_string())
}

/// Read the listing into [snap]. A failed read, or anything but a plain
/// released version, keeps what there is. True when the version presented
/// changed.
pub async fn read_version(snap: &mut Snapshot, listing: &dyn PublicListing, now: u64) -> bool {
    let read = match listing.latest().await {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!("chatgpt catalog: public version listing unavailable: {e}");
            return false;
        }
    };
    if parse_version(&read).is_none() {
        tracing::debug!("chatgpt catalog: ignoring listed version {read:?}");
        return false;
    }
    let before = snap.client_version();
    snap.published_version = Some(read);
    snap.version_read_at = Some(now);
    snap.client_version() != before
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(anyhow::Result<String>);

    #[async_trait]
    impl PublicListing for Fixed {
        async fn latest(&self) -> anyhow::Result<String> {
            match &self.0 {
                Ok(v) => Ok(v.clone()),
                Err(e) => Err(anyhow::anyhow!("{e}")),
            }
        }
    }

    fn ok(v: &str) -> Fixed {
        Fixed(Ok(v.to_string()))
    }

    #[tokio::test]
    async fn a_newer_released_version_is_presented() {
        let mut s = Snapshot::seed();
        assert!(read_version(&mut s, &ok("0.169.1"), 100).await);
        assert_eq!(s.client_version(), "0.169.1");
        assert_eq!(s.version_read_at, Some(100));
    }

    #[tokio::test]
    async fn never_below_the_compiled_value() {
        let mut s = Snapshot::seed();
        assert!(!read_version(&mut s, &ok("0.150.0"), 100).await);
        assert_eq!(s.client_version(), "0.169.0");
    }

    #[tokio::test]
    async fn a_pre_release_or_a_failed_read_keeps_what_there_is() {
        let mut s = Snapshot::seed();
        read_version(&mut s, &ok("0.170.0"), 100).await;
        assert!(!read_version(&mut s, &ok("0.171.0-alpha.2"), 200).await);
        assert!(!read_version(&mut s, &Fixed(Err(anyhow::anyhow!("offline"))), 300).await);
        assert_eq!(s.client_version(), "0.170.0");
        assert_eq!(s.version_read_at, Some(100));
    }

    #[test]
    fn the_listings_answers_are_read_as_measured() {
        let npm = serde_json::json!({"name": "@openai/codex", "version": "0.169.1"});
        assert_eq!(npm_version(&npm).as_deref(), Some("0.169.1"));
        let gh = serde_json::json!({"tag_name": "rust-v0.169.1", "prerelease": false});
        assert_eq!(github_version(&gh).as_deref(), Some("0.169.1"));
        let pre = serde_json::json!({"tag_name": "rust-v0.170.0-alpha.1", "prerelease": true});
        assert_eq!(github_version(&pre), None);
        assert_eq!(npm_version(&serde_json::json!({"version": "0.170.0-alpha.1"})), None);
    }
}

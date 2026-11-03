//! Whether the account keeps a skill's save in the cloud, as last heard —
//! the facts a page and an agent are handed (`GET /api/skill-cloud/{skill}`,
//! `LINGGEN_CLOUD_SAVE`), never a sentence. linggen.dev keeps saves for
//! subscribers only; any other account is answered 402 and plays local-only,
//! quietly: one refusal is believed for [`RECHECK`], so no edge, push or page
//! asks again before then. A copy kept from an earlier subscription is never
//! deleted; `kept_at` says when it was last written.
//!
//! Kept at `~/.linggen/sync/cloud-<skill>.state.json`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::Duration;

use super::files::write_atomic;
use crate::account::cloud::Refusal;

/// How long a refusal is believed before the site is asked again.
pub const RECHECK: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cloud {
    /// Never heard yet.
    #[default]
    Unknown,
    /// The account keeps the save.
    On,
    /// Not subscribed, and no copy is kept.
    Off,
    /// Not subscribed now; a copy from before is kept (`kept_at`) — it may
    /// predate the subscriber gate: no claim the account ever subscribed.
    Lapsed,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CloudState {
    #[serde(default)]
    pub cloud: Cloud,
    /// When the kept copy was last written (ISO, the site's), while lapsed.
    #[serde(default)]
    pub kept_at: Option<String>,
    /// Unix secs of the last sync that reached the account's copy.
    #[serde(default)]
    pub synced_at: Option<u64>,
    /// Unix secs of the first milestone (`further` `rise`) made while
    /// local-only; cleared when the cloud is on again.
    #[serde(default)]
    pub rose_local_at: Option<u64>,
    /// Unix secs the site was last asked.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub checked_at: u64,
    /// Which sign-in the facts belong to (a hash of the token): a new
    /// sign-in asks again at once.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub who: u64,
    /// The save's standing on the `rise` entries, last seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen: Option<Value>,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl CloudState {
    pub fn local_only(&self) -> bool {
        matches!(self.cloud, Cloud::Off | Cloud::Lapsed)
    }

    /// The facts handed out: no bookkeeping.
    pub fn view(&self) -> Value {
        serde_json::json!({
            "cloud": self.cloud,
            "kept_at": self.kept_at,
            "synced_at": self.synced_at,
            "rose_local_at": self.rose_local_at,
        })
    }
}

fn path(skill: &str) -> PathBuf {
    crate::paths::linggen_home()
        .join("sync")
        .join(format!("cloud-{skill}.state.json"))
}

pub fn read(skill: &str) -> CloudState {
    std::fs::read_to_string(path(skill))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn write(skill: &str, s: &CloudState) {
    let text = serde_json::to_string(s).unwrap_or_default();
    if let Err(e) = write_atomic(&path(skill), text.as_bytes()) {
        tracing::debug!("save '{skill}': cloud state not kept: {e:#}");
    }
}

/// The current sign-in, as the state names it.
pub fn who() -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    crate::account::resolve_token().map(|(t, _)| t).hash(&mut h);
    h.finish()
}

/// Whether a refusal heard less than [`RECHECK`] ago, for this sign-in,
/// still stands — the site is not asked.
pub fn backed_off(s: &CloudState, now: u64, who: u64) -> bool {
    s.local_only() && s.who == who && now.saturating_sub(s.checked_at) < RECHECK.as_secs()
}

/// The site refused: local-only, said once in the log when it changes.
pub fn refused(skill: &str, r: &Refusal, now: u64) -> CloudState {
    let mut s = read(skill);
    let cloud = if r.kept { Cloud::Lapsed } else { Cloud::Off };
    if s.cloud != cloud {
        tracing::info!("save '{skill}': not subscribed — kept on this machine only ({cloud:?})");
    }
    s.cloud = cloud;
    s.kept_at = r.kept.then(|| r.updated_at.clone()).flatten();
    s.checked_at = now;
    s.who = who();
    write(skill, &s);
    s
}

/// The account's copy was reached.
pub fn reached(skill: &str, now: u64) {
    let mut s = read(skill);
    if s.local_only() {
        tracing::info!("save '{skill}': the account keeps it again");
    }
    s.cloud = Cloud::On;
    s.kept_at = None;
    s.rose_local_at = None;
    s.synced_at = Some(now);
    s.checked_at = now;
    s.who = who();
    write(skill, &s);
}

/// `LINGGEN_CLOUD_SAVE` for a skill's tool command: the facts, once the
/// skill's save has been heard of at all.
pub fn tool_env(skill: Option<&str>) -> Option<(String, String)> {
    let skill = skill.filter(|s| crate::account::cloud::valid_cloud_name(s))?;
    let s = read(skill);
    if s.cloud == Cloud::Unknown {
        return None;
    }
    Some(("LINGGEN_CLOUD_SAVE".to_string(), s.view().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_is_believed_for_a_while_and_only_for_its_sign_in() {
        let s = CloudState {
            cloud: Cloud::Off,
            checked_at: 1_000,
            who: 7,
            ..Default::default()
        };
        assert!(backed_off(&s, 1_000 + 60, 7));
        assert!(!backed_off(&s, 1_000 + RECHECK.as_secs(), 7), "asked again");
        assert!(
            !backed_off(&s, 1_000 + 60, 8),
            "another sign-in asks at once"
        );
        let on = CloudState {
            cloud: Cloud::On,
            ..s.clone()
        };
        assert!(!backed_off(&on, 1_000 + 60, 7));
    }

    #[test]
    fn the_view_hands_facts_not_bookkeeping() {
        let s = CloudState {
            cloud: Cloud::Lapsed,
            kept_at: Some("2026-11-03T10:00:00Z".into()),
            checked_at: 5,
            who: 9,
            seen: Some(serde_json::json!({"tier": "qi"})),
            ..Default::default()
        };
        assert_eq!(
            s.view(),
            serde_json::json!({"cloud": "lapsed", "kept_at": "2026-11-03T10:00:00Z",
                "synced_at": null, "rose_local_at": null})
        );
    }
}

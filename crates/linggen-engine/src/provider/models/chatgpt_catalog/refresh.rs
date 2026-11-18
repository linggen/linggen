//! Bringing the snapshot up to date: the version first (the backend filters
//! its list by it), then the list, then each family's move. Runs in the
//! background — at start, about once a day, and at sign-in — never on a
//! turn's path. Each source is injected, so tests reach no network.

use super::version::{self, PublicListing};
use super::Snapshot;

/// How long a read stays fresh.
pub const REFRESH_EVERY_SECS: u64 = 24 * 60 * 60;

/// What a refresh reads from.
pub struct Sources<'a> {
    pub listing: &'a dyn PublicListing,
}

/// Whether [snap] wants a refresh at [now]: never read, or read more than a
/// day ago.
pub fn due(snap: &Snapshot, now: u64) -> bool {
    match snap.version_read_at {
        Some(at) => now.saturating_sub(at) >= REFRESH_EVERY_SECS,
        None => true,
    }
}

/// [snap] brought up to date from [src]. Every failed read keeps what there
/// was.
pub async fn refresh(mut snap: Snapshot, src: &Sources<'_>, now: u64) -> Snapshot {
    version::read_version(&mut snap, src.listing, now).await;
    snap
}

/// Unix seconds now.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Refresh the snapshot in force from the real sources and keep it. Only for
/// someone signed in to ChatGPT — without that there is nothing to present
/// a version to, and no request leaves the machine. Unless [force] (a fresh
/// sign-in), only when [`due`]. True when the built-ins changed.
pub async fn refresh_in_force(force: bool) -> bool {
    if !crate::provider::codex_auth::chatgpt_signed_in() {
        return false;
    }
    let before = super::current();
    let now = now();
    if !force && !due(&before, now) {
        return false;
    }
    let listing = version::CodexReleases::new();
    let after = refresh((*before).clone(), &Sources { listing: &listing }, now).await;
    if after == *before {
        return false;
    }
    let changed = after.builtin_ids() != before.builtin_ids();
    super::publish(after);
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_is_due_once_a_day() {
        let mut s = Snapshot::seed();
        assert!(due(&s, 1_000));
        s.version_read_at = Some(1_000);
        assert!(!due(&s, 1_000 + REFRESH_EVERY_SECS - 1));
        assert!(due(&s, 1_000 + REFRESH_EVERY_SECS));
    }
}

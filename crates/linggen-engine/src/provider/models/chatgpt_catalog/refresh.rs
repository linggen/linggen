//! Bringing the snapshot up to date: the version first (the backend filters
//! its list by it), then the list, then each family's move. Runs in the
//! background — at start, about once a day, and at sign-in — never on a
//! turn's path. Each source is injected, so tests reach no network.

use super::listing::{self, Backend};
use super::version::{self, PublicListing};
use super::Snapshot;

/// How long a read stays fresh.
pub const REFRESH_EVERY_SECS: u64 = 24 * 60 * 60;

/// What a refresh reads from.
pub struct Sources<'a> {
    pub listing: &'a dyn PublicListing,
    pub backend: &'a dyn Backend,
}

/// Whether [snap] wants a refresh at [now]: the version or the list never
/// read, or read more than a day ago.
pub fn due(snap: &Snapshot, now: u64) -> bool {
    let stale = |at: Option<u64>| at.is_none_or(|at| now.saturating_sub(at) >= REFRESH_EVERY_SECS);
    stale(snap.version_read_at) || stale(snap.listed_at)
}

/// [snap] brought up to date from [src]: the version, then the list under
/// it. Every failed read keeps what there was.
pub async fn refresh(snap: Snapshot, src: &Sources<'_>, now: u64) -> Snapshot {
    let snap = refresh_version(snap, src.listing, now).await;
    refresh_models(snap, src.backend, now).await
}

async fn refresh_version(mut snap: Snapshot, listing: &dyn PublicListing, now: u64) -> Snapshot {
    version::read_version(&mut snap, listing, now).await;
    snap
}

async fn refresh_models(mut snap: Snapshot, backend: &dyn Backend, now: u64) -> Snapshot {
    listing::read_list(&mut snap, backend, now).await;
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
/// sign-in), only when [`due`]. True when the models to register changed.
pub async fn refresh_in_force(force: bool) -> bool {
    let Some(backend) = listing::ChatgptBackend::signed_in() else {
        return false;
    };
    let before = super::current();
    let now = now();
    if !force && !due(&before, now) {
        return false;
    }
    // The version is put in force before the list is read: every request
    // to the backend presents the one in force.
    let versioned = refresh_version((*before).clone(), &version::CodexReleases::new(), now).await;
    keep_if_changed(&before, versioned.clone());
    let after = refresh_models(versioned, &backend, now).await;
    keep_if_changed(&before, after.clone());
    registered_changed(&before, &after)
}

/// Whether the models a manager registers differ: the built-ins, or the
/// older listed models kept for exact pins.
pub fn registered_changed(before: &Snapshot, after: &Snapshot) -> bool {
    before.builtin_ids() != after.builtin_ids() || before.kept_ids() != after.kept_ids()
}

fn keep_if_changed(before: &Snapshot, after: Snapshot) {
    if after != *before && after != *super::current() {
        super::publish(after);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct Listing(&'static str);

    #[async_trait]
    impl PublicListing for Listing {
        async fn latest(&self) -> anyhow::Result<String> {
            Ok(self.0.to_string())
        }
    }

    /// Answers the measured list and remembers which version asked.
    struct Recording(std::sync::Mutex<Vec<String>>);

    #[async_trait]
    impl Backend for Recording {
        async fn models(&self, v: &str) -> anyhow::Result<serde_json::Value> {
            self.0.lock().unwrap().push(v.to_string());
            Ok(listing::tests::measured_answer())
        }
    }

    #[tokio::test]
    async fn the_list_is_read_under_the_version_just_read() {
        let backend = Recording(Default::default());
        let src = Sources { listing: &Listing("0.169.1"), backend: &backend };
        let s = refresh(Snapshot::seed(), &src, 10).await;
        assert_eq!(*backend.0.lock().unwrap(), vec!["0.169.1".to_string()]);
        assert_eq!(s.listed.len(), 10);
        assert!(!due(&s, 10));
        // The measured list moves no family: each one's newest is current.
        assert_eq!(s.builtin_ids(), Snapshot::seed().builtin_ids());
    }

    #[test]
    fn a_read_is_due_once_a_day() {
        let mut s = Snapshot::seed();
        assert!(due(&s, 1_000));
        s.version_read_at = Some(1_000);
        assert!(due(&s, 1_000), "the list was never read");
        s.listed_at = Some(1_000);
        assert!(!due(&s, 1_000 + REFRESH_EVERY_SECS - 1));
        assert!(due(&s, 1_000 + REFRESH_EVERY_SECS));
    }
}

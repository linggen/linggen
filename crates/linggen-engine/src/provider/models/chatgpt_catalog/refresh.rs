//! Bringing the snapshot up to date: the version first (the backend filters
//! its list by it), then the list, then each family's move. Runs in the
//! background — at start, about once a day, and at sign-in — never on a
//! turn's path. Each source is injected, so tests reach no network.

use super::listing::{self, Backend};
use super::promote::{self, Prober};
use super::version::{self, PublicListing};
use super::{Snapshot, Store};

/// How long a read stays fresh.
pub const REFRESH_EVERY_SECS: u64 = 24 * 60 * 60;

/// How long a pass that could not read everything waits before the next try.
pub const RETRY_AFTER_FAILURE_SECS: u64 = 4 * 60 * 60;

/// What a refresh reads from.
pub struct Sources<'a> {
    pub listing: &'a dyn PublicListing,
    pub backend: &'a dyn Backend,
    pub prober: &'a dyn Prober,
}

/// Whether [snap] wants a refresh at [now]: the version or the list never
/// read, or read more than a day ago — and no pass tried in the last
/// [`RETRY_AFTER_FAILURE_SECS`], so sources that cannot be reached are asked
/// again after the back-off, not on every tick.
pub fn due(snap: &Snapshot, now: u64) -> bool {
    let older_than = |at: Option<u64>, secs: u64| at.is_none_or(|at| now.saturating_sub(at) >= secs);
    let stale = older_than(snap.version_read_at, REFRESH_EVERY_SECS)
        || older_than(snap.listed_at, REFRESH_EVERY_SECS);
    stale && older_than(snap.pass_tried_at, RETRY_AFTER_FAILURE_SECS)
}

/// [snap] brought up to date from [src]: the version, then the list under
/// it, then each family's move to a newer model that answers. Every failed
/// read keeps what there was.
pub async fn refresh(snap: Snapshot, src: &Sources<'_>, now: u64) -> Snapshot {
    let snap = refresh_version(snap, src.listing, now).await;
    refresh_models(snap, src.backend, src.prober, now).await
}

async fn refresh_version(mut snap: Snapshot, listing: &dyn PublicListing, now: u64) -> Snapshot {
    version::read_version(&mut snap, listing, now).await;
    snap
}

async fn refresh_models(
    mut snap: Snapshot,
    backend: &dyn Backend,
    prober: &dyn Prober,
    now: u64,
) -> Snapshot {
    listing::read_list(&mut snap, backend, now).await;
    promote::promote(&mut snap, prober, now).await;
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
    let (Some(backend), Some(probe_client)) =
        (listing::ChatgptBackend::signed_in(), listing::signed_in_client())
    else {
        return false;
    };
    let listing = version::CodexReleases::new();
    let prober = promote::ChatgptProber::new(probe_client);
    let src = Sources { listing: &listing, backend: &backend, prober: &prober };
    run_pass(&super::InForce, &src, force, now()).await
}

/// One pass at a time: a sign-in's refresh and the timer's never overlap.
static PASS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// One refresh against [store]: the version is kept (put in force) before
/// the list is read — every request to the backend presents the one in
/// force — then the list and each family's move. A choice the person made
/// meanwhile (staying) is carried over, never overwritten. True when the
/// models to register changed.
pub async fn run_pass(store: &dyn Store, src: &Sources<'_>, force: bool, now: u64) -> bool {
    let _one = PASS.lock().await;
    let before = store.current();
    if !force && !due(&before, now) {
        return false;
    }
    let mut attempt = (*before).clone();
    attempt.pass_tried_at = Some(now);
    let versioned = refresh_version(attempt, src.listing, now).await;
    keep(store, &before, &versioned);
    let after = refresh_models(versioned, src.backend, src.prober, now).await;
    let kept = keep(store, &before, &after);
    registered_changed(&before, &kept)
}

/// Keep [after] in [store], with any choice made since [before] carried
/// over.
fn keep(store: &dyn Store, before: &Snapshot, after: &Snapshot) -> std::sync::Arc<Snapshot> {
    store.modify(&mut |in_force| {
        let mut next = after.clone();
        carry_choices(before, in_force, &mut next);
        (next != *in_force).then_some(next)
    })
}

/// A family's stay the person changed while the pass ran wins — as long as
/// it still names the same previous model.
fn carry_choices(before: &Snapshot, in_force: &Snapshot, next: &mut Snapshot) {
    for fam in next.families.iter_mut() {
        let (Some(was), Some(now)) = (before.family(&fam.name), in_force.family(&fam.name)) else {
            continue;
        };
        if was.stay != now.stay && fam.previous == now.previous {
            fam.stay = now.stay;
        }
    }
}

/// Whether the models a manager registers differ: the built-ins, or the
/// older listed models kept for exact pins.
pub fn registered_changed(before: &Snapshot, after: &Snapshot) -> bool {
    before.builtin_ids() != after.builtin_ids() || before.kept_ids() != after.kept_ids()
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

    struct Answers;

    #[async_trait]
    impl Prober for Answers {
        async fn answers(&self, _slug: &str) -> bool {
            panic!("the measured list has nothing newer to try")
        }
    }

    #[tokio::test]
    async fn the_list_is_read_under_the_version_just_read() {
        let backend = Recording(Default::default());
        let src = Sources { listing: &Listing("0.169.1"), backend: &backend, prober: &Answers };
        let s = refresh(Snapshot::seed(), &src, 10).await;
        assert_eq!(*backend.0.lock().unwrap(), vec!["0.169.1".to_string()]);
        assert_eq!(s.listed.len(), 10);
        assert!(!due(&s, 10));
        // The measured list moves no family: each one's newest is current.
        assert_eq!(s.builtin_ids(), Snapshot::seed().builtin_ids());
    }

    /// A test's own store.
    struct Held(std::sync::Mutex<std::sync::Arc<Snapshot>>);

    impl Held {
        fn new(s: Snapshot) -> Self {
            Self(std::sync::Mutex::new(std::sync::Arc::new(s)))
        }
    }

    impl Store for Held {
        fn current(&self) -> std::sync::Arc<Snapshot> {
            self.0.lock().unwrap().clone()
        }
        fn modify(
            &self,
            change: &mut dyn FnMut(&Snapshot) -> Option<Snapshot>,
        ) -> std::sync::Arc<Snapshot> {
            let mut g = self.0.lock().unwrap();
            if let Some(next) = change(&g) {
                *g = std::sync::Arc::new(next);
            }
            g.clone()
        }
    }

    /// Answers the measured list plus a newer Sol; on its read it records
    /// the version in force, and (when asked) the person presses Stay.
    struct Midway<'a> {
        store: &'a Held,
        in_force_at_read: std::sync::Mutex<Option<String>>,
        press_stay: bool,
    }

    #[async_trait]
    impl Backend for Midway<'_> {
        async fn models(&self, _v: &str) -> anyhow::Result<serde_json::Value> {
            *self.in_force_at_read.lock().unwrap() = Some(self.store.current().client_version());
            if self.press_stay {
                self.store.modify(&mut |cur| {
                    let mut next = cur.clone();
                    next.set_stay("sol", true).ok()?;
                    Some(next)
                });
            }
            let mut answer = listing::tests::measured_answer();
            answer["models"].as_array_mut().unwrap().insert(
                0,
                serde_json::json!({"slug": "gpt-7-sol", "visibility": "list", "priority": 0}),
            );
            Ok(answer)
        }
    }

    struct Yes;

    #[async_trait]
    impl Prober for Yes {
        async fn answers(&self, _slug: &str) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn a_pass_puts_the_version_in_force_before_the_list_and_moves_a_family() {
        let store = Held::new(Snapshot::seed());
        let backend = Midway { store: &store, in_force_at_read: Default::default(), press_stay: false };
        let src = Sources { listing: &Listing("0.170.0"), backend: &backend, prober: &Yes };
        assert!(run_pass(&store, &src, false, 100).await);
        assert_eq!(backend.in_force_at_read.lock().unwrap().as_deref(), Some("0.170.0"));
        assert_eq!(store.current().builtin_ids()[0], "gpt-7-sol");
        assert!(!run_pass(&store, &src, false, 101).await, "not due again");
    }

    #[tokio::test]
    async fn staying_pressed_during_a_pass_is_not_overwritten() {
        let mut moved = Snapshot::seed();
        let sol = moved.family_mut("sol").unwrap();
        sol.previous = Some("gpt-6.1-sol".into());
        sol.history.push("gpt-6.1-sol".into());
        sol.moved_at = Some(50);
        let store = Held::new(moved);
        let backend = Midway { store: &store, in_force_at_read: Default::default(), press_stay: true };
        // The newer Sol is tried and refuses; the press lands mid-pass.
        struct No;
        #[async_trait]
        impl Prober for No {
            async fn answers(&self, _slug: &str) -> bool {
                false
            }
        }
        let src = Sources { listing: &Listing("0.169.1"), backend: &backend, prober: &No };
        run_pass(&store, &src, true, 100).await;
        let kept = store.current();
        assert!(kept.family("sol").unwrap().stay, "the press survived the pass");
        assert_eq!(kept.builtin_ids()[0], "gpt-6.1-sol");
        assert_eq!(kept.listed_at, Some(100));
    }

    struct Offline;

    #[async_trait]
    impl PublicListing for Offline {
        async fn latest(&self) -> anyhow::Result<String> {
            anyhow::bail!("unreachable")
        }
    }

    /// Where the public listing cannot be reached, a pass is not run again
    /// on every hourly tick: the attempt is stamped, and the next one waits
    /// out the back-off.
    #[tokio::test]
    async fn a_failed_read_retries_after_the_back_off_not_every_hour() {
        let store = Held::new(Snapshot::seed());
        let backend = Recording(Default::default());
        let src = Sources { listing: &Offline, backend: &backend, prober: &Answers };
        run_pass(&store, &src, false, 1_000).await;
        assert_eq!(backend.0.lock().unwrap().len(), 1, "the first pass ran");
        assert!(store.current().version_read_at.is_none(), "the read failed");
        run_pass(&store, &src, false, 1_000 + 3_600).await;
        assert_eq!(backend.0.lock().unwrap().len(), 1, "an hour later it waits");
        run_pass(&store, &src, false, 1_000 + RETRY_AFTER_FAILURE_SECS).await;
        assert_eq!(backend.0.lock().unwrap().len(), 2, "after the back-off it tries again");
        assert!(RETRY_AFTER_FAILURE_SECS < REFRESH_EVERY_SECS);
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

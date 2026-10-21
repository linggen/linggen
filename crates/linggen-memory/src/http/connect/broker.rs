//! The one open request to connect, and what became of recent ones.
//!
//! In memory only: a request lives two minutes, and a daemon restart simply
//! makes the asking machine ask again. One request is pending at a time — a
//! new one replaces the old, as the engine's pairing does — so the console
//! never shows a queue of strangers.

use serde::Serialize;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a request waits for a click.
pub const TTL: Duration = Duration::from_secs(120);
/// How long a decided request can still be asked about.
const KEEP_OUTCOME: Duration = Duration::from_secs(600);
/// Requests one address may open per window.
const RATE_MAX: usize = 6;
const RATE_WINDOW: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Serialize)]
pub struct Pending {
    pub id: String,
    pub device_name: String,
    pub device_id: String,
    pub ip: IpAddr,
    #[serde(skip)]
    pub at: Instant,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    Pending,
    Denied,
    Expired,
    /// The token, the first time it is asked for; `None` after.
    Allowed(Option<String>),
}

enum Outcome {
    Allowed(Option<String>),
    Denied,
}

#[derive(Default)]
struct Inner {
    pending: Option<Pending>,
    outcomes: HashMap<String, (Outcome, Instant)>,
    hits: HashMap<IpAddr, Vec<Instant>>,
}

#[derive(Default)]
pub struct Broker {
    inner: Mutex<Inner>,
}

impl Broker {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Count a request from `ip`; false once it is over the limit.
    pub fn admit(&self, ip: IpAddr, now: Instant) -> bool {
        let mut inner = self.lock();
        let hits = inner.hits.entry(ip).or_default();
        hits.retain(|t| now.duration_since(*t) < RATE_WINDOW);
        if hits.len() >= RATE_MAX {
            return false;
        }
        hits.push(now);
        true
    }

    /// Open a request, replacing whatever was pending.
    pub fn open(
        &self,
        device_name: String,
        device_id: String,
        ip: IpAddr,
        now: Instant,
    ) -> Pending {
        let pending = Pending {
            id: uuid::Uuid::new_v4().to_string(),
            device_name,
            device_id,
            ip,
            at: now,
        };
        let mut inner = self.lock();
        inner
            .outcomes
            .retain(|_, (_, at)| now.duration_since(*at) < KEEP_OUTCOME);
        inner.pending = Some(pending.clone());
        pending
    }

    /// The live request, if any.
    pub fn pending(&self, now: Instant) -> Option<Pending> {
        self.lock().pending.clone().filter(|p| is_live(p, now))
    }

    /// Take the live request with this id off the table, to decide it.
    pub fn take(&self, id: &str, now: Instant) -> Option<Pending> {
        let mut inner = self.lock();
        let live = inner
            .pending
            .as_ref()
            .is_some_and(|p| p.id == id && is_live(p, now));
        live.then(|| inner.pending.take()).flatten()
    }

    pub fn allow(&self, id: String, token: String, now: Instant) {
        self.lock()
            .outcomes
            .insert(id, (Outcome::Allowed(Some(token)), now));
    }

    pub fn deny(&self, id: String, now: Instant) {
        self.lock().outcomes.insert(id, (Outcome::Denied, now));
    }

    /// What became of a request. The token goes out once, then is gone.
    pub fn status(&self, id: &str, now: Instant) -> Status {
        let mut inner = self.lock();
        if let Some((outcome, _)) = inner.outcomes.get_mut(id) {
            return match outcome {
                Outcome::Allowed(token) => Status::Allowed(token.take()),
                Outcome::Denied => Status::Denied,
            };
        }
        let live = inner
            .pending
            .as_ref()
            .is_some_and(|p| p.id == id && is_live(p, now));
        if live {
            Status::Pending
        } else {
            Status::Expired
        }
    }
}

fn is_live(p: &Pending, now: Instant) -> bool {
    now.duration_since(p.at) < TTL
}

/// Seconds left on a request, for the console's countdown.
pub fn expires_in(p: &Pending, now: Instant) -> u64 {
    TTL.saturating_sub(now.duration_since(p.at)).as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip() -> IpAddr {
        "192.168.20.31".parse().unwrap()
    }

    fn open(b: &Broker, now: Instant) -> Pending {
        b.open("DS242".into(), "dev-1".into(), ip(), now)
    }

    #[test]
    fn a_new_request_replaces_the_old_one() {
        let b = Broker::default();
        let now = Instant::now();
        let first = open(&b, now);
        let second = open(&b, now);
        assert_ne!(first.id, second.id);
        assert_eq!(b.pending(now).unwrap().id, second.id);
        assert_eq!(b.status(&first.id, now), Status::Expired);
        assert!(b.take(&first.id, now).is_none());
    }

    #[test]
    fn a_request_expires_unanswered() {
        let b = Broker::default();
        let now = Instant::now();
        let p = open(&b, now);
        let later = now + TTL + Duration::from_secs(1);
        assert!(b.pending(later).is_none());
        assert_eq!(b.status(&p.id, later), Status::Expired);
        assert!(b.take(&p.id, later).is_none());
    }

    /// The token reaches the request that asked, once.
    #[test]
    fn the_token_is_handed_out_once() {
        let b = Broker::default();
        let now = Instant::now();
        let p = open(&b, now);
        assert_eq!(b.status(&p.id, now), Status::Pending);
        b.take(&p.id, now).unwrap();
        b.allow(p.id.clone(), "cafe".into(), now);
        assert_eq!(b.status(&p.id, now), Status::Allowed(Some("cafe".into())));
        assert_eq!(b.status(&p.id, now), Status::Allowed(None));
        assert_eq!(b.status("someone-else", now), Status::Expired);
    }

    #[test]
    fn a_denied_request_says_so() {
        let b = Broker::default();
        let now = Instant::now();
        let p = open(&b, now);
        b.take(&p.id, now).unwrap();
        b.deny(p.id.clone(), now);
        assert_eq!(b.status(&p.id, now), Status::Denied);
        assert!(b.pending(now).is_none());
    }

    #[test]
    fn request_ids_are_uuid_v4() {
        let b = Broker::default();
        let p = open(&b, Instant::now());
        let parsed = uuid::Uuid::parse_str(&p.id).unwrap();
        assert_eq!(parsed.get_version_num(), 4);
    }

    #[test]
    fn one_address_cannot_flood_the_console() {
        let b = Broker::default();
        let now = Instant::now();
        for _ in 0..RATE_MAX {
            assert!(b.admit(ip(), now));
        }
        assert!(!b.admit(ip(), now));
        assert!(b.admit("192.168.20.32".parse().unwrap(), now));
        assert!(b.admit(ip(), now + RATE_WINDOW));
    }
}

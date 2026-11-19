//! Wrong pairing codes, counted across codes: per peer address and over
//! all peers, each with a wait that grows with every wrong guess. A new code
//! clears neither count; a right code clears its peer's. A count is
//! forgotten after an hour without a wrong guess.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

/// Wrong guesses a peer gets before it must wait.
const PEER_FREE: u32 = 3;
/// Wrong guesses all peers together get before every peer must wait.
const ALL_FREE: u32 = 10;
/// The longest wait.
const MAX_WAIT: Duration = Duration::from_secs(15 * 60);
/// A count with no wrong guess for this long starts over.
const FORGET: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Copy)]
struct Tally {
    wrong: u32,
    last: Instant,
}

impl Tally {
    /// When the next guess may come: one second after the first wrong guess
    /// past `free`, doubling with each one after, up to [`MAX_WAIT`].
    fn open_at(&self, free: u32) -> Instant {
        if self.wrong < free {
            return self.last;
        }
        let doublings = (self.wrong - free).min(16);
        self.last + Duration::from_secs(1u64 << doublings).min(MAX_WAIT)
    }

    fn fresh(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last) < FORGET
    }
}

#[derive(Default)]
pub(super) struct Guesses {
    by_peer: HashMap<IpAddr, Tally>,
    all: Option<Tally>,
}

impl Guesses {
    /// How long `peer` must still wait before its guess is heard; `None`
    /// when it may guess now.
    pub(super) fn wait(&mut self, peer: IpAddr, now: Instant) -> Option<Duration> {
        self.forget(now);
        let peer_open = self.by_peer.get(&peer).map(|t| t.open_at(PEER_FREE));
        let all_open = self.all.map(|t| t.open_at(ALL_FREE));
        let open = peer_open.into_iter().chain(all_open).max()?;
        (open > now).then(|| open - now)
    }

    pub(super) fn wrong(&mut self, peer: IpAddr, now: Instant) {
        let bump = |t: Option<Tally>| Tally {
            wrong: t.map_or(0, |t| t.wrong) + 1,
            last: now,
        };
        let peer_tally = bump(self.by_peer.get(&peer).copied());
        self.by_peer.insert(peer, peer_tally);
        self.all = Some(bump(self.all));
    }

    pub(super) fn right(&mut self, peer: IpAddr) {
        self.by_peer.remove(&peer);
    }

    fn forget(&mut self, now: Instant) {
        self.by_peer.retain(|_, t| t.fresh(now));
        self.all = self.all.filter(|t| t.fresh(now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(last: u8) -> IpAddr {
        IpAddr::from([192, 0, 2, last])
    }

    #[test]
    fn a_peer_waits_longer_with_each_wrong_guess_past_its_share() {
        let mut g = Guesses::default();
        let t0 = Instant::now();
        for _ in 0..PEER_FREE {
            assert_eq!(g.wait(ip(1), t0), None);
            g.wrong(ip(1), t0);
        }
        assert_eq!(g.wait(ip(1), t0), Some(Duration::from_secs(1)));
        g.wrong(ip(1), t0);
        assert_eq!(g.wait(ip(1), t0), Some(Duration::from_secs(2)));
        // Another peer is not held by this one's count alone.
        assert_eq!(g.wait(ip(2), t0), None);
        // The wait passes; a right guess clears the peer's count.
        assert_eq!(g.wait(ip(1), t0 + Duration::from_secs(2)), None);
        g.right(ip(1));
        assert_eq!(g.wait(ip(1), t0), None);
    }

    #[test]
    fn wrong_guesses_from_many_peers_hold_every_peer() {
        let mut g = Guesses::default();
        let t0 = Instant::now();
        for n in 0..ALL_FREE {
            g.wrong(ip(n as u8 + 10), t0);
        }
        assert_eq!(g.wait(ip(200), t0), Some(Duration::from_secs(1)));
    }

    #[test]
    fn the_wait_is_capped_and_an_hour_quiet_forgets() {
        let mut g = Guesses::default();
        let t0 = Instant::now();
        for _ in 0..40 {
            g.wrong(ip(1), t0);
        }
        assert_eq!(g.wait(ip(1), t0), Some(MAX_WAIT));
        assert_eq!(g.wait(ip(1), t0 + FORGET), None);
    }
}

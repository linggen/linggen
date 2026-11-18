//! The standing a peer is judged by at its check points — the session
//! streams, the media channel, the events and page state it is sent — kept
//! with the device it was resolved for (`control::standing_for`).
//!
//! Resolved again when the device the peer named changes, and at most
//! [`FRESH`] after the last time, so a level set on this Mac reaches a live
//! channel without a reconnect.

use std::time::{Duration, Instant};

use crate::server::person::Standing;
use crate::server::rtc::UserContext;

/// How long a resolved standing is taken as current.
const FRESH: Duration = Duration::from_secs(2);

pub(super) struct PeerStanding {
    device: Option<String>,
    standing: Standing,
    at: Instant,
}

impl PeerStanding {
    pub(super) fn new(user_ctx: &UserContext, device: Option<String>) -> Self {
        Self {
            standing: super::control::standing_for(user_ctx, device.as_deref()),
            device,
            at: Instant::now(),
        }
    }

    /// The standing last resolved.
    pub(super) fn resolved(&self) -> &Standing {
        &self.standing
    }

    /// The standing for `device` (the one the peer has named, if any).
    /// `true` beside it when the person changed since the last call.
    pub(super) fn current(
        &mut self,
        user_ctx: &UserContext,
        device: Option<String>,
    ) -> (Standing, bool) {
        if self.device == device && self.at.elapsed() < FRESH {
            return (self.standing.clone(), false);
        }
        let next = super::control::standing_for(user_ctx, device.as_deref());
        let moved = next.person() != self.standing.person();
        *self = Self {
            device,
            standing: next,
            at: Instant::now(),
        };
        (self.standing.clone(), moved)
    }
}

/// Whose sessions and events a peer is given when it is not the owner: a
/// room consumer's are its own account's, any other peer's its person's —
/// never the channel's user id, which for a device on the LAN is the
/// owner's. `None` for the owner, who is given everything.
pub(super) fn seen_as(user_ctx: &UserContext, standing: &Standing) -> Option<String> {
    if user_ctx.is_consumer() {
        return Some(user_ctx.user_id.clone());
    }
    (!standing.is_owner()).then(|| standing.person().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::rtc::{Reach, UserContext};

    /// A device on the LAN arrives on the owner's context; what it is given
    /// is its person's, not that context's user id. The owner's own page is
    /// given everything; a room consumer its own account's.
    #[test]
    fn a_peer_is_given_its_persons_never_the_channels() {
        let lan = UserContext {
            reached_by: Reach::Lan,
            ..UserContext::owner(Some("u-owner".into()))
        };
        let unnamed = PeerStanding::new(&lan, None);
        assert_eq!(
            seen_as(&lan, unnamed.resolved()).as_deref(),
            Some(crate::server::rtc::NO_ONE)
        );
        let unknown = PeerStanding::new(&lan, Some("not-paired".into()));
        assert_eq!(
            seen_as(&lan, unknown.resolved()).as_deref(),
            Some(crate::server::rtc::NO_ONE)
        );
        let here = UserContext {
            reached_by: Reach::ThisMachine,
            ..UserContext::owner(Some("u-owner".into()))
        };
        assert_eq!(
            seen_as(&here, PeerStanding::new(&here, None).resolved()),
            None
        );
        let room = UserContext::consumer(
            "u-room".into(),
            "browser".into(),
            crate::server::rtc::UserPermission::Read,
            None,
            None,
        );
        assert_eq!(
            seen_as(&room, PeerStanding::new(&room, None).resolved()).as_deref(),
            Some("u-room")
        );
    }

    /// A newly named device is resolved at once, and says the person moved.
    #[test]
    fn naming_a_device_resolves_it_again() {
        let ctx = UserContext::paired();
        let mut s = PeerStanding::new(&ctx, None);
        let (_, moved) = s.current(&ctx, None);
        assert!(!moved);
        let (named, _) = s.current(&ctx, Some("not-paired".into()));
        assert_eq!(
            named.level(),
            crate::engine::permission::PermissionMode::Chat
        );
    }
}

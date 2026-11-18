//! Who a person is: the owner, another account, a device that named no
//! account, or no one. Made only in `server::person`, which resolves people;
//! anywhere else a person is had from a resolved standing, the owner
//! (`person::owner`), or read back from the string a session, a memory row,
//! a URL or an event keeps ([`Person::from_wire`]).
//!
//! The strings stay what they were before this type: the owner is this
//! Mac's account id (`__local__` while it is signed out, and none or an empty
//! one reads as the owner too); another account is its id; a device that
//! named no account is `device:<id>`; no one is `__paired__` (and
//! `__pairing__`, a peer mid-pairing, reads as no one too). Only this file
//! turns a person into those strings and back, and only this file reads this
//! Mac's account to say who the owner is ([`mac_account`]).

/// The id the owner goes by while this Mac is signed out.
const LOCAL: &str = "__local__";
/// A peer that has not said which device it is.
const NO_ONE: &str = "__paired__";
/// A peer that has scanned the QR and is not paired yet: no one as well.
const PAIRING: &str = "__pairing__";
/// What a device that named no account is kept under, before its id.
const DEVICE: &str = "device:";

/// Who a request, a session, a day or a memory row is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Person(Kind);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    /// This Mac's owner: this machine, an approved machine, the owner's own
    /// phone.
    Owner,
    /// Another person, by their account's id.
    Account(String),
    /// A device that named no account (a phone signed out, or one claiming
    /// this Mac's account unverified), by its device id.
    Device(String),
    /// A peer that has not said which device it is.
    NoOne,
}

/// This Mac's account id, as its account file keeps it: the one read of the
/// account that says who the owner is.
pub(crate) fn mac_account() -> Option<String> {
    crate::account::load_account().and_then(|a| a.user_id)
}

impl Person {
    pub(super) fn owner() -> Self {
        Person(Kind::Owner)
    }

    pub(super) fn no_one() -> Self {
        Person(Kind::NoOne)
    }

    pub(super) fn device(device_id: &str) -> Self {
        Person(Kind::Device(device_id.to_string()))
    }

    pub(super) fn account(id: &str) -> Self {
        Person(Kind::Account(id.to_string()))
    }

    /// The person a kept string names (a session's `user_id`, a request's,
    /// a stage's), with this Mac's account as it is now.
    pub(crate) fn from_wire(id: Option<&str>) -> Self {
        Self::from_wire_with(id, mac_account().as_deref())
    }

    /// [`Self::from_wire`] with this Mac's account `mac`: none, an empty id,
    /// [`LOCAL`] and `mac` are the owner; [`NO_ONE`] and [`PAIRING`] no one;
    /// `device:<id>` that device; any other id that account.
    pub(super) fn from_wire_with(id: Option<&str>, mac: Option<&str>) -> Self {
        let Some(id) = id.map(str::trim).filter(|id| !id.is_empty()) else {
            return Self::owner();
        };
        if id == LOCAL || mac == Some(id) {
            return Self::owner();
        }
        if id == NO_ONE || id == PAIRING {
            return Self::no_one();
        }
        match id.strip_prefix(DEVICE) {
            Some(device) => Self::device(device),
            None => Self::account(id),
        }
    }

    /// The string this person is kept and sent as, with this Mac's account
    /// as it is now.
    pub(crate) fn wire(&self) -> String {
        match self.0 {
            Kind::Owner => self.wire_with(mac_account().as_deref()),
            _ => self.wire_with(None),
        }
    }

    /// [`Self::wire`] with this Mac's account `mac`.
    pub(super) fn wire_with(&self, mac: Option<&str>) -> String {
        match &self.0 {
            Kind::Owner => mac.unwrap_or(LOCAL).to_string(),
            Kind::Account(id) => id.clone(),
            Kind::Device(id) => stamp_of(id),
            Kind::NoOne => NO_ONE.to_string(),
        }
    }

    /// The owner.
    pub(crate) fn is_owner(&self) -> bool {
        self.0 == Kind::Owner
    }

    /// No one: a peer that has not said which device it is.
    pub(crate) fn is_no_one(&self) -> bool {
        self.0 == Kind::NoOne
    }

    /// A device that named no account.
    pub(crate) fn is_device(&self) -> bool {
        matches!(self.0, Kind::Device(_))
    }

    /// The person a day is kept under (`None`: the owner's day); `None` for
    /// no one, who is given no day.
    pub(crate) fn day_key(&self) -> Option<Option<String>> {
        match &self.0 {
            Kind::Owner => Some(None),
            Kind::NoOne => None,
            _ => Some(Some(self.wire_with(None))),
        }
    }

    /// A person for a test elsewhere, read from its kept string with this
    /// Mac signed in as `u-owner`.
    #[cfg(test)]
    pub(crate) fn for_test(id: Option<&str>) -> Self {
        Self::from_wire_with(id, Some("u-owner"))
    }

    /// The owner, for a test elsewhere.
    #[cfg(test)]
    pub(crate) fn test_owner() -> Self {
        Self::owner()
    }

    /// No one, for a test elsewhere.
    #[cfg(test)]
    pub(crate) fn test_no_one() -> Self {
        Self::no_one()
    }
}

/// The id a device that named no account is kept under: its rows in
/// memory, its day.
pub(crate) fn stamp_of(device_id: &str) -> String {
    format!("{DEVICE}{device_id}")
}

/// The id a peer mid-pairing is given on its channel: no one.
pub(crate) fn pairing_wire() -> &'static str {
    PAIRING
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every string a person is kept as reads as it did before the type, and
    /// each person is written back as the same string.
    #[test]
    fn a_person_reads_and_writes_the_strings_kept_today() {
        let mac = Some("u-owner");
        let read = |id: Option<&str>| Person::from_wire_with(id, mac);
        for owner in [None, Some(""), Some("  "), Some("__local__"), Some("u-owner"), Some(" u-owner ")] {
            assert_eq!(read(owner), Person::owner(), "{owner:?}");
        }
        assert_eq!(read(Some("__paired__")), Person::no_one());
        assert_eq!(read(Some("__pairing__")), Person::no_one());
        assert_eq!(read(Some("device:dev-1")), Person::device("dev-1"));
        assert_eq!(read(Some("u-friend")), Person::account("u-friend"));
        assert_eq!(Person::owner().wire_with(mac), "u-owner");
        assert_eq!(Person::owner().wire_with(None), "__local__");
        assert_eq!(Person::no_one().wire_with(mac), "__paired__");
        assert_eq!(Person::device("dev-1").wire_with(mac), "device:dev-1");
        assert_eq!(Person::account("u-friend").wire_with(mac), "u-friend");
        assert_eq!(pairing_wire(), "__pairing__");
    }

    /// Signed out, the Mac's owner is `__local__` and every account another
    /// person.
    #[test]
    fn signed_out_the_owner_is_this_machine() {
        assert_eq!(Person::from_wire_with(Some("__local__"), None), Person::owner());
        assert_eq!(Person::from_wire_with(Some("u-owner"), None), Person::account("u-owner"));
    }

    /// The day a person is kept under: none for the owner, their string for
    /// another, no day for no one.
    #[test]
    fn a_persons_day() {
        assert_eq!(Person::owner().day_key(), Some(None));
        assert_eq!(Person::account("u-friend").day_key(), Some(Some("u-friend".into())));
        assert_eq!(Person::device("dev-1").day_key(), Some(Some("device:dev-1".into())));
        assert_eq!(Person::no_one().day_key(), None);
    }
}

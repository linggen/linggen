//! The paired-device store — `~/.linggen/paired-devices.json`, the rows the
//! pairing handshake mints and everything else reads (the LAN gate, the
//! WebRTC tunnel's attribution, perception's device names).
//!
//! Plain state: no HTTP here. The handshake lives in `server::api::pair`.
//!
//! Each row has a **kind**: a `phone` (the code-pairing ceremony) or a
//! `machine` (the owner's other computer, approved with Allow). Rows written
//! before the kind existed are classified once on load (`classify`).

use rand::RngExt;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Who is holding the phone — the linggen.dev account signed in on it, as the
/// phone claims it on `identify`.
///
/// A claim names a person; it never makes the phone the owner by itself. The
/// owner's standing needs the claim to be verified ([`PairedDevice::is_owners`]).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AccountRef {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// When the phone last told us this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

/// Stamped onto every record a phone causes the Mac to write. `device` is the
/// Mac-minted row id, which a phone cannot choose; `account` is what the phone
/// claims and is absent while it is signed out.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Actor {
    pub device: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

/// What a paired row is. Decides whose memory it reaches
/// (`memory_host::caller`): a machine is the owner's, a phone its person's.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Phone,
    Machine,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PairedDevice {
    pub id: String,
    pub name: String,
    pub secret: String,
    pub created_at: i64,
    /// `None` only until `classify` has run on a row from before kinds; read
    /// it through [`PairedDevice::kind`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<DeviceKind>,
    /// The account signed in on this phone, last time it said so. A phone can
    /// sign in and out long after pairing, so this is refreshed on connect —
    /// never only at pair time.
    #[serde(default)]
    pub account: Option<AccountRef>,
    /// The account the relay vouched for this phone: linggen.dev said, when
    /// the phone dialled, that the account credential it sent owns this Mac's
    /// instance, and the phone's claim named that account. Kept only while the
    /// claim stays that account — a different claim, or signing out, clears
    /// it ([`refresh_account`]); a reconnect with no relay keeps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_account: Option<String>,
    /// The level this row held before levels belonged to a person
    /// (`server::people`): read once, when the people file is first made
    /// from the rows, and cleared then. Nothing sets it, and nothing else
    /// reads it.
    #[serde(default, rename = "permission", skip_serializing_if = "Option::is_none")]
    pub legacy_permission: Option<crate::engine::permission::PermissionMode>,
    /// The contract number this phone said it speaks on its last
    /// `identify`; unset, it has said none (contract 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<u32>,
    /// Stable per-phone install id. Re-pairing the same phone replaces its row
    /// (matched on this) rather than stacking duplicates. Optional so rows
    /// written before this field, and older apps that don't send one, still load.
    #[serde(default)]
    pub device_id: Option<String>,
    /// Per-device settings the Mac owns and the phone pulls (Settings → Phone,
    /// one-way Mac→phone). E.g. `{"models": [ids]}`. Preserved across re-pair.
    #[serde(default)]
    pub settings: serde_json::Map<String, serde_json::Value>,
    /// The relay credential we asked linggen.dev to issue for this phone, kept
    /// so revoking the phone can revoke that too.
    ///
    /// Without it, unpairing removed the row here and left the credential live
    /// on the relay: the phone lost the LAN but kept working from anywhere,
    /// which is the opposite of what Revoke means.
    #[serde(default)]
    pub relay_grant: Option<String>,
    /// The credential the row this one replaced was carrying, so a re-pair can
    /// retire it. Never persisted — it exists only between `commit_device` and
    /// the caller that issues the new one.
    #[serde(skip)]
    pub superseded_grant: Option<String>,
}

impl PairedDevice {
    /// The row's kind; a row nobody classified is a phone, the narrower grant.
    pub fn kind(&self) -> DeviceKind {
        self.kind.unwrap_or(DeviceKind::Phone)
    }

    /// The account this phone claims, when the relay has vouched for it.
    pub fn verified_claim(&self) -> Option<&str> {
        let claim = self.account.as_ref().map(|a| a.id.as_str())?;
        (self.verified_account.as_deref() == Some(claim)).then_some(claim)
    }

    /// Whether this row is the owner's: a machine approved with Allow, or a
    /// phone whose verified claim is this Mac's own account (`mac_account`).
    /// The one place a device is decided to be the owner's.
    pub fn is_owners(&self, mac_account: Option<&str>) -> bool {
        if self.kind() == DeviceKind::Machine {
            return true;
        }
        let mac = mac_account.filter(|m| !m.is_empty());
        mac.is_some() && self.verified_claim() == mac
    }
}

/// The header a paired device presents its token in.
pub const DEVICE_HEADER: &str = "x-linggen-device";

/// Header the WebRTC tunnel stamps on a request it forwards for an identified
/// peer. The phone speaks only WebRTC, so its calls reach the router from
/// loopback with no device token — this is how a handler still knows which
/// phone is asking.
///
/// Trust: only the tunnel writes it, and anything else able to reach loopback
/// is already the Mac's owner. It routes and attributes; it never authorizes.
pub const ACTOR_DEVICE_HEADER: &str = "x-linggen-actor-device";

/// Header the WebRTC tunnel stamps on EVERY request it re-issues to loopback.
/// Loopback alone does not mean "someone at this Mac" once the tunnel is
/// involved. Its value says what the tunnel knows of a peer that has not
/// named a device: [`TUNNEL_LOCAL`], [`TUNNEL_OWNER`], or anything else.
pub const TUNNEL_HEADER: &str = "x-linggen-tunnel";

/// Header the WebRTC tunnel and the LAN gate stamp on every request they
/// send on for a caller from off this machine: the level it was judged at
/// (`server::person::send_judged`), for a handler that judges within its
/// route (a skill tool's tier, a device revoking only itself). Set over
/// whatever the caller sent; absent, the request is from this machine and
/// at admin. It can only lower what a request may do ([`caller_level`]).
pub const LEVEL_HEADER: &str = "x-linggen-level";

/// The level a request stands at, from [`LEVEL_HEADER`]: absent is this
/// machine, at admin; a value that is not a level is chat.
pub fn caller_level(headers: &axum::http::HeaderMap) -> crate::engine::permission::PermissionMode {
    use crate::engine::permission::PermissionMode;
    let Some(value) = headers.get(LEVEL_HEADER) else {
        return PermissionMode::Admin;
    };
    value
        .to_str()
        .ok()
        .and_then(|v| serde_json::from_value(serde_json::Value::String(v.to_string())).ok())
        .unwrap_or(PermissionMode::Chat)
}

/// The peer's channel was opened from this machine (the local web UI, the
/// app shell).
pub const TUNNEL_LOCAL: &str = "local";

/// The peer reached the relay as the owner's own account.
pub const TUNNEL_OWNER: &str = "owner";

fn devices_path() -> PathBuf {
    crate::paths::linggen_home().join("paired-devices.json")
}

mod store;
pub use store::write_atomic;

static STORE: std::sync::LazyLock<store::DeviceStore> =
    std::sync::LazyLock::new(|| store::DeviceStore::new(devices_path()));

/// The paired devices — served from memory; see [`store`].
pub fn load_devices() -> Vec<PairedDevice> {
    STORE.load(migrate_rows)
}

/// Re-read the file on the next call, now — after another writer (memory's
/// Allow) has added a row whose token must work at once.
pub fn reload_devices() {
    STORE.forget();
}

/// Read-modify-write the device list under the store's writer lock, persisted
/// atomically.
pub fn modify_devices<R>(f: impl FnOnce(&mut Vec<PairedDevice>) -> R) -> std::io::Result<R> {
    STORE.modify(migrate_rows, f)
}

/// Rows written before the models default existed carry no "models" key at
/// all, and re-pairs preserve that forever — seed those once. An explicit
/// list (even an emptied one) is the owner's curation and stays untouched.
/// Rows from before kinds get theirs (`classify`).
fn migrate_rows(devices: &mut [PairedDevice]) -> bool {
    let mut seeded = classify(devices, &connect_allowed_ids());
    for d in devices {
        if !d.settings.contains_key("models") {
            d.settings.append(&mut default_device_settings());
            seeded = true;
        }
        seeded |= migrate_retired_models(&mut d.settings);
    }
    seeded
}

/// Give each row from before kinds its kind, by how it was made: the Allow
/// ceremony recorded every machine it approved in
/// `memory/connect-devices.json` (`connect_allowed`); every other row came
/// from the code-pairing ceremony, a phone. True when a row changed.
fn classify(devices: &mut [PairedDevice], machines: &[String]) -> bool {
    let mut changed = false;
    for d in devices.iter_mut().filter(|d| d.kind.is_none()) {
        d.kind = Some(if machines.contains(&d.id) {
            DeviceKind::Machine
        } else {
            DeviceKind::Phone
        });
        changed = true;
    }
    changed
}

/// The row ids the Allow ceremony approved, from its own record. Missing or
/// unreadable: none — every unclassified row then stays a phone.
fn connect_allowed_ids() -> Vec<String> {
    #[derive(Deserialize)]
    struct Allowed {
        id: String,
    }
    let path = crate::paths::linggen_home()
        .join("memory")
        .join("connect-devices.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<Allowed>>(&t).ok())
        .map(|rows| rows.into_iter().map(|a| a.id).collect())
        .unwrap_or_default()
}

/// A phone's allow-list follows the ChatGPT catalog in force.
fn migrate_retired_models(settings: &mut serde_json::Map<String, serde_json::Value>) -> bool {
    follow_catalog(settings, &crate::provider::models::chatgpt_catalog::current())
}

/// Move each older ChatGPT id in a phone's allow-list to its family's model
/// in [catalog] (deduped), so the phone never keeps offering a model the
/// family has moved on from. True when the list changed.
fn follow_catalog(
    settings: &mut serde_json::Map<String, serde_json::Value>,
    catalog: &crate::provider::models::chatgpt_catalog::Snapshot,
) -> bool {
    let Some(list) = settings.get_mut("models").and_then(|v| v.as_array_mut()) else {
        return false;
    };
    let follow = |x: &serde_json::Value| x.as_str().and_then(|id| catalog.follow(id));
    if !list.iter().any(|x| follow(x).is_some()) {
        return false;
    }
    let mut seen = std::collections::HashSet::new();
    *list = std::mem::take(list)
        .into_iter()
        .map(|x| follow(&x).map(serde_json::Value::from).unwrap_or(x))
        .filter(|x| seen.insert(x.to_string()))
        .collect();
    true
}

/// Move every paired device's allow-list along with the catalog in force —
/// what a family move asks for. True when a list changed.
pub fn follow_chatgpt_catalog() -> std::io::Result<bool> {
    modify_devices(|devices| {
        devices
            .iter_mut()
            .fold(false, |changed, d| migrate_retired_models(&mut d.settings) | changed)
    })
}

/// What a brand-new phone's allow-list starts with: the zero-setup Linggen
/// Cloud model, plus the ChatGPT built-in — `oauth` kind, so the phone offers
/// it and the user signs in to ChatGPT on the device to use it. `cloud` needs
/// only the account token the phone adopts on pair, so the Model picker is
/// usable the moment pairing finishes instead of empty until the Mac owner
/// curates it. An owner's explicit edit on the Mac sticks across re-pairs.
pub(crate) fn default_device_settings() -> serde_json::Map<String, serde_json::Value> {
    let mut settings = serde_json::Map::new();
    settings.insert(
        "models".to_string(),
        serde_json::json!([
            crate::provider::models::LINGGEN_CLOUD_MODEL_ID,
            crate::provider::models::chatgpt_catalog::primary_id(),
        ]),
    );
    settings
}

/// Mint a token for a freshly-confirmed device and persist it. A phone that
/// sends a stable `device_id` replaces its own prior row (re-pairing refreshes
/// the token/name in place instead of stacking duplicates); without one — older
/// apps — it appends, as before.
pub fn commit_device(
    name: String,
    device_id: Option<String>,
    account: Option<AccountRef>,
) -> std::io::Result<PairedDevice> {
    let device = modify_devices(|devices| {
        // Carry the Mac-owned settings across a re-pair so pulling from the Mac
        // doesn't reset to defaults when a phone re-scans.
        let prior = device_id
            .as_deref()
            .and_then(|did| devices.iter().find(|d| d.device_id.as_deref() == Some(did)));
        let carried = prior.map(|d| d.settings.clone());
        let superseded_grant = prior.and_then(|d| d.relay_grant.clone());
        let device = PairedDevice {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            account: account.map(stamp_account),
            // Vouched for on a later `identify`, never at pairing.
            verified_account: None,
            legacy_permission: None,
            // Said on the next `identify`.
            contract: None,
            secret: random_hex(24),
            created_at: chrono::Utc::now().timestamp(),
            kind: Some(DeviceKind::Phone),
            device_id: device_id.clone(),
            settings: carried.unwrap_or_else(default_device_settings),
            // Filled in by the caller once the relay has issued one.
            relay_grant: None,
            superseded_grant,
        };
        if let Some(did) = &device_id {
            devices.retain(|d| d.device_id.as_deref() != Some(did.as_str()));
        }
        devices.push(device.clone());
        device
    })?;
    // A new device on this Mac is a change to the world, and the kind a user
    // asks about later ("when did I pair this?").
    crate::perception::activity::record("user", "system", "pair", Some(device.name.clone()));
    Ok(device)
}

/// Stamp "when the phone told us" so a stale claim is visible as stale.
fn stamp_account(mut a: AccountRef) -> AccountRef {
    a.at = Some(chrono::Utc::now().to_rfc3339());
    a
}

/// What to call the person behind a phone, wherever one is named — the paired
/// device list, and anything the Mac records about who did something.
///
/// A phone that is signed out is still somebody: pairing does not require an
/// account, and requiring one to be *named* would leave every action a guest
/// takes attributed to nobody, which reads as a bug rather than as a guest.
/// The stored row keeps the truth — `account` stays absent — and only the
/// label fills in.
pub const GUEST_LABEL: &str = "Guest";

pub fn person_label(account: &Option<AccountRef>) -> String {
    let Some(a) = account else {
        return GUEST_LABEL.to_string();
    };
    a.name
        .as_deref()
        .or(a.email.as_deref())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(GUEST_LABEL)
        .to_string()
}

/// Resolve a device token into the actor to stamp on records it causes.
/// `None` for an unknown token — an unattributed record beats a wrong one.
pub fn actor_for_token(token: &str) -> Option<Actor> {
    device_by_token(token).map(|d| Actor {
        device: d.id,
        account: d.account.map(|a| a.id),
    })
}

/// What a connect-time identity refresh found.
pub struct Identified {
    pub actor: Actor,
    /// The account this device just gained — it had none before this call
    /// and has one now. A sign-in, and the one moment its device-stamped
    /// memory rows should be handed to the person.
    pub signed_in: Option<AccountRef>,
    /// The account the relay vouched for on this call, when the device was
    /// not already verified as it: the moment rows it wrote before become the
    /// owner's.
    pub verified_now: Option<String>,
}

/// Refresh the account a phone claims. Sign-in and sign-out both happen long
/// after pairing, so the connect-time claim wins over the pair-time one —
/// including `None`, which is how a sign-out is recorded. `vouched_for` is
/// the account the relay vouched for on this channel (this Mac's own, when
/// the relay said the phone's account owns the instance).
pub fn set_device_account(
    token: &str,
    account: Option<AccountRef>,
    vouched_for: Option<&str>,
    contract: u32,
) -> Option<Identified> {
    device_by_token(token)?;
    let result = modify_devices(|devices| {
        let d = devices.iter_mut().find(|d| token_matches(d, token))?;
        // The contract it speaks, as it said on this `identify`.
        d.contract = Some(contract);
        Some(refresh_account(d, account, vouched_for))
    });
    match result {
        Ok(found) => found,
        Err(e) => {
            tracing::warn!("[pair] could not persist device account: {e}");
            None
        }
    }
}

/// The claim replaces the last one. A verification is kept only while the
/// claim stays the account it was made for; it is made only when the relay
/// vouched for exactly the account claimed. A claim alone never verifies.
fn refresh_account(
    d: &mut PairedDevice,
    account: Option<AccountRef>,
    vouched_for: Option<&str>,
) -> Identified {
    let next = account.map(stamp_account);
    let claim = next.as_ref().map(|a| a.id.clone());
    let changed = d.account.as_ref().map(|a| a.id.clone()) != claim;
    let signed_in = match (&d.account, &next) {
        (None, Some(a)) => Some(a.clone()),
        _ => None,
    };
    let was_verified = d.verified_claim().map(str::to_string);
    d.account = next;
    if d.verified_account != claim {
        d.verified_account = None;
    }
    if let (Some(v), Some(c)) = (vouched_for, claim.as_deref()) {
        if v == c {
            d.verified_account = Some(c.to_string());
        }
    }
    let verified_now = d
        .verified_claim()
        .filter(|v| was_verified.as_deref() != Some(*v))
        .map(str::to_string);
    let actor = Actor {
        device: d.id.clone(),
        account: d.account.as_ref().map(|a| a.id.clone()),
    };
    if changed {
        tracing::info!(
            "[pair] device '{}' is now {}",
            actor.device,
            person_label(&d.account)
        );
    }
    Identified {
        actor,
        signed_in,
        verified_now,
    }
}

/// Constant-time: a token compare must not leak how many bytes matched.
pub(crate) fn token_matches(d: &PairedDevice, token: &str) -> bool {
    !token.is_empty() && crate::util::ct_eq(&d.secret, token)
}

/// The device that owns a token, if any — lets `/api/pair/me` identify the caller.
pub fn device_by_token(token: &str) -> Option<PairedDevice> {
    (!token.is_empty())
        .then(|| load_devices().into_iter().find(|d| token_matches(d, token)))
        .flatten()
}

/// What the user calls this device. `None` when the id belongs to nothing —
/// a device unpaired mid-connection, say.
///
/// Ids are for matching; a person hears "Alex's iPhone", so anything the
/// agent says out loud resolves through here first.
pub fn device_name(id: &str) -> Option<String> {
    load_devices()
        .into_iter()
        .find(|d| d.id == id)
        .map(|d| d.name)
}

/// The LAN gate's check: does any paired device own this token?
pub fn is_valid_device_token(token: &str) -> bool {
    load_devices().iter().any(|d| token_matches(d, token))
}

pub fn random_hex(bytes: usize) -> String {
    let mut rng = rand::rng();
    (0..bytes)
        .map(|_| format!("{:02x}", rng.random::<u8>()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, kind: Option<DeviceKind>) -> PairedDevice {
        PairedDevice {
            id: id.into(),
            name: id.into(),
            secret: format!("s-{id}"),
            created_at: 0,
            kind,
            ..Default::default()
        }
    }

    fn claim(id: &str) -> Option<AccountRef> {
        Some(AccountRef {
            id: id.into(),
            email: None,
            name: None,
            at: None,
        })
    }

    /// A claim of the Mac's own account is not the owner until the relay
    /// vouches for it; vouched, it is; a vouch for another account than the
    /// one claimed verifies nothing.
    #[test]
    fn only_a_vouched_claim_of_the_macs_account_is_the_owner() {
        let mut d = row("p", Some(DeviceKind::Phone));
        refresh_account(&mut d, claim("u-owner"), None);
        assert!(!d.is_owners(Some("u-owner")), "an unverified claim is not the owner");
        assert_eq!(d.verified_claim(), None);
        let id = refresh_account(&mut d, claim("u-owner"), Some("u-other"));
        assert!(!d.is_owners(Some("u-owner")));
        assert_eq!(id.verified_now, None);
        let id = refresh_account(&mut d, claim("u-owner"), Some("u-owner"));
        assert!(d.is_owners(Some("u-owner")), "vouched for the account it claims");
        assert_eq!(id.verified_now.as_deref(), Some("u-owner"));
        assert!(!d.is_owners(Some("u-someone")), "the owner is this Mac's account only");
        assert!(!d.is_owners(None), "a Mac signed out has no owner's phone");
    }

    /// A verification survives a reconnect with no relay (the LAN, no
    /// internet) while the claim stays; a changed claim or a sign-out clears
    /// it, and claiming the account again does not bring it back.
    #[test]
    fn a_verification_lasts_while_the_claim_does() {
        let mut d = row("p", Some(DeviceKind::Phone));
        refresh_account(&mut d, claim("u-owner"), Some("u-owner"));
        let again = refresh_account(&mut d, claim("u-owner"), None);
        assert!(d.is_owners(Some("u-owner")), "kept on a reconnect with no relay");
        assert_eq!(again.verified_now, None, "not verified anew");
        refresh_account(&mut d, claim("u-friend"), None);
        assert!(!d.is_owners(Some("u-owner")));
        refresh_account(&mut d, claim("u-owner"), None);
        assert!(!d.is_owners(Some("u-owner")), "a changed claim cleared it");
        refresh_account(&mut d, claim("u-owner"), Some("u-owner"));
        refresh_account(&mut d, None, None);
        assert_eq!(d.verified_account, None, "signing out cleared it");
    }

    /// A machine approved with Allow is the owner's with no verification.
    #[test]
    fn an_approved_machine_is_the_owners() {
        assert!(row("m", Some(DeviceKind::Machine)).is_owners(Some("u-owner")));
        assert!(row("m", Some(DeviceKind::Machine)).is_owners(None));
        assert!(!row("p", None).is_owners(Some("u-owner")));
    }

    /// A verification written by the Mac is kept on disk; a row from before
    /// it reads as unverified.
    #[test]
    fn a_verification_round_trips() {
        let mut d = row("p", Some(DeviceKind::Phone));
        refresh_account(&mut d, claim("u-owner"), Some("u-owner"));
        let back: PairedDevice = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert!(back.is_owners(Some("u-owner")));
        let old: PairedDevice = serde_json::from_str(
            r#"{"id":"a","name":"n","secret":"s","created_at":0,"account":{"id":"u-owner"}}"#,
        )
        .unwrap();
        assert!(!old.is_owners(Some("u-owner")));
    }

    /// Rows from before kinds: the Allow ceremony's own list names the
    /// machines; every other row is a phone; a row already classified keeps
    /// its kind.
    #[test]
    fn old_rows_are_classified_by_how_they_were_made() {
        let mut rows = vec![
            row("phone-1", None),
            row("mac-2", None),
            row("kept", Some(DeviceKind::Machine)),
        ];
        assert!(classify(&mut rows, &["mac-2".to_string()]));
        assert_eq!(rows[0].kind(), DeviceKind::Phone);
        assert_eq!(rows[1].kind(), DeviceKind::Machine);
        assert_eq!(rows[2].kind(), DeviceKind::Machine);
        assert!(!classify(&mut rows, &[]), "classified once");
    }

    #[test]
    fn a_row_without_a_kind_reads_as_a_phone_and_round_trips() {
        let old: PairedDevice =
            serde_json::from_str(r#"{"id":"a","name":"n","secret":"s","created_at":0}"#).unwrap();
        assert_eq!(old.kind, None);
        assert_eq!(old.kind(), DeviceKind::Phone);
        let machine = row("m", Some(DeviceKind::Machine));
        let text = serde_json::to_string(&machine).unwrap();
        assert!(text.contains(r#""kind":"machine""#));
    }

    #[test]
    fn a_phone_allow_list_follows_a_family_that_moved() {
        use crate::provider::models::chatgpt_catalog::{tests::measured_list, Listed, Snapshot};
        let mut snap = Snapshot::seed();
        snap.listed = measured_list();
        snap.listed.insert(0, Listed { slug: "gpt-7-sol".into(), priority: 0, context_window: None });
        let mut s = serde_json::Map::new();
        s.insert("models".into(), serde_json::json!(["deepseek-flash", "gpt-6.2-sol"]));
        assert!(!follow_catalog(&mut s, &snap), "not moved until it answered");
        let sol = snap.family_mut("sol").unwrap();
        sol.previous = Some("gpt-6.2-sol".into());
        sol.current = "gpt-7-sol".into();
        assert!(follow_catalog(&mut s, &snap));
        assert_eq!(s["models"], serde_json::json!(["deepseek-flash", "gpt-7-sol"]));
    }

    #[test]
    fn a_phone_allow_list_follows_the_gpt_bump() {
        let mut s = serde_json::Map::new();
        s.insert(
            "models".into(),
            serde_json::json!([
                "deepseek-flash",
                "gpt-5.6-terra",
                "gpt-5.6-luna",
                "gpt-5.6-sol",
                "gpt-6-sol"
            ]),
        );
        assert!(migrate_retired_models(&mut s));
        assert_eq!(
            s["models"],
            serde_json::json!([
                "deepseek-flash",
                "gpt-6.1-terra",
                "gpt-6-luna",
                "gpt-6.2-sol"
            ])
        );
        assert!(!migrate_retired_models(&mut s));
    }
}

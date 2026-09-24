//! The paired-device store — `~/.linggen/paired-devices.json`, the rows the
//! pairing handshake mints and everything else reads (the LAN gate, the
//! WebRTC tunnel's attribution, perception's device names).
//!
//! Plain state: no HTTP here. The handshake lives in `server::api::pair`.

use rand::RngExt;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Who is holding the phone — the linggen.dev account signed in on it.
///
/// Phone-asserted: the Mac records what the phone claims rather than verifying
/// it against linggen.dev. That is enough for a household ledger ("whose photo
/// is this"), and deliberately not enough to be a security boundary.
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

#[derive(Serialize, Deserialize, Clone)]
pub struct PairedDevice {
    pub id: String,
    pub name: String,
    pub secret: String,
    pub created_at: i64,
    /// The account signed in on this phone, last time it said so. A phone can
    /// sign in and out long after pairing, so this is refreshed on connect —
    /// never only at pair time.
    #[serde(default)]
    pub account: Option<AccountRef>,
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

fn devices_path() -> PathBuf {
    crate::paths::linggen_home().join("paired-devices.json")
}

mod store;

static STORE: std::sync::LazyLock<store::DeviceStore> =
    std::sync::LazyLock::new(|| store::DeviceStore::new(devices_path()));

/// The paired devices — served from memory; see [`store`].
pub fn load_devices() -> Vec<PairedDevice> {
    STORE.load(migrate_rows)
}

/// Read-modify-write the device list under the store's writer lock, persisted
/// atomically.
pub(crate) fn modify_devices<R>(f: impl FnOnce(&mut Vec<PairedDevice>) -> R) -> std::io::Result<R> {
    STORE.modify(migrate_rows, f)
}

/// Rows written before the models default existed carry no "models" key at
/// all, and re-pairs preserve that forever — seed those once. An explicit
/// list (even an emptied one) is the owner's curation and stays untouched.
fn migrate_rows(devices: &mut [PairedDevice]) -> bool {
    let mut seeded = false;
    for d in devices {
        if !d.settings.contains_key("models") {
            d.settings.append(&mut default_device_settings());
            seeded = true;
        }
        seeded |= migrate_retired_models(&mut d.settings);
    }
    seeded
}

/// A GPT generation bump moves each retired ChatGPT id in a phone's
/// allow-list to its successor (deduped), so the phone never keeps offering
/// a model the backend no longer serves. True when the list changed.
fn migrate_retired_models(settings: &mut serde_json::Map<String, serde_json::Value>) -> bool {
    use crate::provider::models::chatgpt_successor;
    let Some(list) = settings.get_mut("models").and_then(|v| v.as_array_mut()) else {
        return false;
    };
    if !list
        .iter()
        .any(|x| x.as_str().is_some_and(|id| chatgpt_successor(id).is_some()))
    {
        return false;
    }
    let mut seen = std::collections::HashSet::new();
    *list = std::mem::take(list)
        .into_iter()
        .map(|x| match x.as_str().and_then(chatgpt_successor) {
            Some(next) => serde_json::Value::from(next),
            None => x,
        })
        .filter(|x| seen.insert(x.to_string()))
        .collect();
    true
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
            crate::provider::models::CHATGPT_BUILTIN_MODEL_ID,
        ]),
    );
    settings
}

/// Mint a token for a freshly-confirmed device and persist it. A phone that
/// sends a stable `device_id` replaces its own prior row (re-pairing refreshes
/// the token/name in place instead of stacking duplicates); without one — older
/// apps — it appends, as before.
pub(crate) fn commit_device(
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
            secret: random_hex(24),
            created_at: chrono::Utc::now().timestamp(),
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
}

/// Refresh the account a phone claims. Sign-in and sign-out both happen long
/// after pairing, so the connect-time claim wins over the pair-time one —
/// including `None`, which is how a sign-out is recorded.
pub fn set_device_account(token: &str, account: Option<AccountRef>) -> Option<Identified> {
    device_by_token(token)?;
    let result = modify_devices(|devices| {
        let d = devices.iter_mut().find(|d| token_matches(d, token))?;
        Some(refresh_account(d, account))
    });
    match result {
        Ok(found) => found,
        Err(e) => {
            tracing::warn!("[pair] could not persist device account: {e}");
            None
        }
    }
}

fn refresh_account(d: &mut PairedDevice, account: Option<AccountRef>) -> Identified {
    let next = account.map(stamp_account);
    let changed = d.account.as_ref().map(|a| a.id.clone()) != next.as_ref().map(|a| a.id.clone());
    let signed_in = match (&d.account, &next) {
        (None, Some(a)) => Some(a.clone()),
        _ => None,
    };
    d.account = next;
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
    Identified { actor, signed_in }
}

/// Constant-time: a token compare must not leak how many bytes matched.
pub(crate) fn token_matches(d: &PairedDevice, token: &str) -> bool {
    !token.is_empty() && crate::util::ct_eq(&d.secret, token)
}

/// The device that owns a token, if any — lets `/api/pair/me` identify the caller.
pub(crate) fn device_by_token(token: &str) -> Option<PairedDevice> {
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

pub(crate) fn random_hex(bytes: usize) -> String {
    let mut rng = rand::rng();
    (0..bytes)
        .map(|_| format!("{:02x}", rng.random::<u8>()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phone_allow_list_follows_the_gpt_bump() {
        let mut s = serde_json::Map::new();
        s.insert(
            "models".into(),
            serde_json::json!([
                "deepseek-flash",
                "gpt-5.6-terra",
                "gpt-5.6-luna",
                "gpt-5.6-sol"
            ]),
        );
        assert!(migrate_retired_models(&mut s));
        assert_eq!(
            s["models"],
            serde_json::json!(["deepseek-flash", "gpt-6-luna", "gpt-6-sol"])
        );
        assert!(!migrate_retired_models(&mut s));
    }
}

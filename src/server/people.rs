//! The people of this Mac's household: what each person who is not the
//! owner may use here — their level, which apps, which of this Mac's models,
//! and how many tokens a day. Kept in `people.json`, by person (an account's
//! id, or a device's own stamp while its phone is signed out: the strings
//! `person::Person` writes), never by device.
//!
//! This file is the one reader and writer of `people.json` (lock:
//! `the_people_file_is_read_and_written_here_alone`). What it answers, each
//! other place asks and none decides again:
//! - the level a person stands at ([`People::level_of`], asked by
//!   `person::standing_of`);
//! - the ceiling a turn of theirs runs under ([`People::ceiling_of`], asked
//!   by `chat::policy::turn_policy`);
//! - whether they may use an app ([`People::may_use_app`], asked by every
//!   way into an app and every list of apps they are shown);
//! - whether their next turn may start ([`People::refusal_for`]), counted
//!   against the day's tokens they have used.
//!
//! The owner has no row and no limit. A person nobody set anything for is
//! at read, with every app the owner has not kept to themselves, and none
//! of this Mac's models. A room's guest keeps the room's own config as its
//! ceiling (`chat::policy`), the same type, built in the same place.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::engine::permission::PermissionMode;
use crate::server::person::{Person, DEVICE_DEFAULT, MOST_FOR_ANOTHER};
use crate::state_fs::devices::PairedDevice;

/// What a person's turn is held to besides their level: the apps and the
/// Mac's models it may use (`SessionPolicy::person`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Ceiling {
    pub skills: Vec<String>,
    pub models: Vec<String>,
}

/// Why a person's turn does not start: a fact the companion words
/// (`status` in the chat's reply), never a sentence made here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The owner has shared none of this Mac's models with them.
    NoModel,
    /// They have used the day's tokens (the day turns at UTC midnight, as a
    /// room's budget does).
    OverLimit,
}

impl Refusal {
    /// The word the chat's reply carries.
    pub(crate) fn status(self) -> &'static str {
        match self {
            Refusal::NoModel => "no_model",
            Refusal::OverLimit => "over_limit",
        }
    }
}

/// What happened to a person's settings when their phone signed in
/// (`person_changed`): the People page says it in one line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Arrival {
    /// The account had none: what the owner chose for the signed-out phone
    /// is now the account's.
    Moved,
    /// The account had its own: they stand, and the phone's are dropped.
    Kept,
}

/// One person's settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PersonSettings {
    /// The most they may do (chat, read or edit); unset, read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<PermissionMode>,
    /// The apps they may use; unset, all (but those the owner keeps).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apps: Option<Vec<String>>,
    /// The models of this Mac their turns may run on; none, none.
    #[serde(default)]
    pub models: Vec<String>,
    /// The tokens a day they may use on this Mac's models; unset, no limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_tokens: Option<u64>,
    /// How their settings came to be the account's, when a sign-in did it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrival: Option<Arrival>,
}

/// What the owner keeps to themselves.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OwnerSide {
    /// Apps no one else uses.
    #[serde(default)]
    pub mine_only: Vec<String>,
}

/// The file: the owner's side, and the settings of each person.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct People {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    pub owner: OwnerSide,
    /// By the person's kept string (`Person::wire`).
    #[serde(default)]
    people: BTreeMap<String, PersonSettings>,
}

const VERSION: u32 = 1;

impl People {
    /// The settings of `person`: theirs, else what nobody having set
    /// anything means. The owner and no one have no row.
    pub(crate) fn settings_of(&self, person: &Person) -> PersonSettings {
        if !person.is_another() {
            return PersonSettings::default();
        }
        self.people.get(&person.wire()).cloned().unwrap_or_default()
    }

    /// The level `person` stands at when they are not the owner.
    pub(crate) fn level_of(&self, person: &Person) -> PermissionMode {
        self.settings_of(person)
            .level
            .unwrap_or(DEVICE_DEFAULT)
            .min(MOST_FOR_ANOTHER)
    }

    /// Whether `person` may use app `app`: the owner any; anyone else every
    /// app the owner has not kept, within their own list when they have one.
    /// The one answer every way into an app and every list asks.
    pub(crate) fn may_use_app(&self, person: &Person, app: &str) -> bool {
        if person.is_owner() {
            return true;
        }
        if self.owner.mine_only.iter().any(|a| a == app) {
            return false;
        }
        self.settings_of(person)
            .apps
            .is_none_or(|list| list.iter().any(|a| a == app))
    }

    /// The ceiling of `person`'s turn: of the apps `installed`, those they
    /// may use; and their models, with each one's successor too.
    pub(crate) fn ceiling_of(&self, person: &Person, installed: &[String]) -> Ceiling {
        Ceiling {
            skills: installed
                .iter()
                .filter(|app| self.may_use_app(person, app))
                .cloned()
                .collect(),
            models: crate::server::rtc::room_config::with_successors(self.settings_of(person).models),
        }
    }

    /// Whether `person`'s turns may run on this Mac's model `model`: the owner
    /// any; anyone else those they were given.
    pub(crate) fn may_run_model(&self, person: &Person, model: &str) -> bool {
        person.is_owner() || self.ceiling_of(person, &[]).models.iter().any(|m| m == model)
    }

    /// Why `person`'s next turn must not start, given the tokens `used` they
    /// have used today; `None` for the owner and for a turn that may.
    pub(crate) fn refusal_for(&self, person: &Person, used: u64) -> Option<Refusal> {
        if !person.is_another() {
            return None;
        }
        let settings = self.settings_of(person);
        if settings.models.is_empty() {
            return Some(Refusal::NoModel);
        }
        let over = settings.daily_tokens.is_some_and(|limit| used >= limit);
        over.then_some(Refusal::OverLimit)
    }

    /// The daily limit of `person`, for the budget store's check.
    pub(crate) fn limit_of(&self, person: &Person) -> Option<u64> {
        self.settings_of(person).daily_tokens
    }

    /// Set what `patch` names of `person`'s settings; `Err` words the
    /// first thing wrong with it, and nothing is changed.
    pub(crate) fn set(
        &mut self,
        person: &Person,
        patch: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), &'static str> {
        if !person.is_another() {
            return Err("only another person has settings");
        }
        let mut next = self.settings_of(person);
        next.apply(patch)?;
        self.people.insert(person.wire(), next);
        Ok(())
    }

    /// Set the apps the owner keeps to themselves.
    pub(crate) fn keep_for_owner(&mut self, apps: Vec<String>) {
        self.owner.mine_only = tidy(apps);
    }

    /// Move what was set for `from` (a signed-out phone's person) to `to`
    /// (who it became): another phone's stamp, kept as it was; an account,
    /// which keeps its own when it has some and takes the phone's when not.
    /// What happened, for an account.
    fn carry(&mut self, from: &str, to: &Person) -> Option<Arrival> {
        let moved = self.people.remove(from)?;
        if !to.is_another() {
            return None;
        }
        let key = to.wire();
        if to.is_device() {
            self.people.insert(key, moved);
            return None;
        }
        let arrival = match self.people.contains_key(&key) {
            true => Arrival::Kept,
            false => Arrival::Moved,
        };
        let mut settings = self.people.remove(&key).unwrap_or(moved);
        settings.arrival = Some(arrival);
        self.people.insert(key, settings);
        Some(arrival)
    }
}

impl PersonSettings {
    /// Apply the keys of `patch` — `level`, `apps` (null: all), `models`,
    /// `daily_tokens` (null: none), `arrival` (null: seen) — or none of them.
    fn apply(&mut self, patch: &serde_json::Map<String, serde_json::Value>) -> Result<(), &'static str> {
        let mut next = self.clone();
        for (key, value) in patch {
            match key.as_str() {
                "level" => next.level = Some(level_of_word(value)?),
                "apps" => next.apps = optional(value, strings)?,
                "models" => next.models = strings(value)?,
                "daily_tokens" => next.daily_tokens = optional(value, count)?,
                "arrival" if value.is_null() => next.arrival = None,
                _ => return Err("not a setting of a person"),
            }
        }
        *self = next;
        Ok(())
    }
}

/// A level the owner may give another person: chat, read or edit.
fn level_of_word(value: &serde_json::Value) -> Result<PermissionMode, &'static str> {
    let level: PermissionMode = serde_json::from_value(value.clone()).map_err(|_| "not a level")?;
    match level > MOST_FOR_ANOTHER {
        true => Err("admin is the owner's own; set chat, read or edit"),
        false => Ok(level),
    }
}

fn optional<T>(
    value: &serde_json::Value,
    read: fn(&serde_json::Value) -> Result<T, &'static str>,
) -> Result<Option<T>, &'static str> {
    match value.is_null() {
        true => Ok(None),
        false => read(value).map(Some),
    }
}

fn strings(value: &serde_json::Value) -> Result<Vec<String>, &'static str> {
    let list = value.as_array().ok_or("a list of names")?;
    let names: Option<Vec<String>> = list.iter().map(|v| v.as_str().map(str::to_string)).collect();
    names.map(tidy).ok_or("a list of names")
}

fn count(value: &serde_json::Value) -> Result<u64, &'static str> {
    value.as_u64().ok_or("a whole number")
}

/// A list of names without blanks or repeats, in the order given.
fn tidy(names: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    names
        .into_iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty() && seen.insert(n.clone()))
        .collect()
}

/// The people file as the device rows made it: each person's level is the
/// lowest any of their rows held (a row with none held read), never more
/// than any one of them had. A person none of whose rows held a level, the
/// owner's rows and no one's are not in it.
fn first_people(devices: &[PairedDevice], mac: Option<&str>) -> People {
    // Per person: whether any of their rows held a level, and the lowest.
    let mut held: BTreeMap<String, (bool, PermissionMode)> = BTreeMap::new();
    for row in devices {
        let person = crate::server::person::of_device(row, mac);
        if !person.is_another() {
            continue;
        }
        let level = row.legacy_permission.unwrap_or(DEVICE_DEFAULT).min(MOST_FOR_ANOTHER);
        let (any, lowest) = held.entry(person.wire()).or_insert((false, level));
        *any |= row.legacy_permission.is_some();
        *lowest = (*lowest).min(level);
    }
    People {
        version: VERSION,
        owner: OwnerSide::default(),
        people: held
            .into_iter()
            .filter(|(_, (any, _))| *any)
            .map(|(key, (_, level))| {
                let settings = PersonSettings {
                    level: Some(level),
                    ..PersonSettings::default()
                };
                (key, settings)
            })
            .collect(),
    }
}

// ---- the file ----------------------------------------------------------

/// How long a read is trusted before the file's stamp is looked at again.
const RECHECK: Duration = Duration::from_secs(1);

type Stamp = Option<(SystemTime, u64)>;

struct Cached {
    people: Arc<People>,
    stamp: Stamp,
    checked: Instant,
}

/// The people file, cached; every write goes through [`Store::modify`].
struct Store {
    path: PathBuf,
    cache: Mutex<Option<Cached>>,
}

fn stamp_of(path: &std::path::Path) -> Stamp {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

impl Store {
    fn at(path: PathBuf) -> Self {
        Self {
            path,
            cache: Mutex::new(None),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Cached>> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The people as they are: from memory, the file looked at again when it
    /// is older than [`RECHECK`]. A file that does not read gives the safe
    /// default (everyone at read, no models) and is never written over.
    fn current(&self, origin: &dyn Origin) -> Arc<People> {
        let mut cache = self.lock();
        if let Some(c) = cache.as_ref().filter(|c| c.checked.elapsed() < RECHECK) {
            return c.people.clone();
        }
        let stamp = stamp_of(&self.path);
        if let Some(c) = cache.as_mut().filter(|c| c.stamp == stamp) {
            c.checked = Instant::now();
            return c.people.clone();
        }
        let people = Arc::new(self.read(origin).unwrap_or_default());
        let stamp = stamp_of(&self.path);
        *cache = Some(Cached {
            people: people.clone(),
            stamp,
            checked: Instant::now(),
        });
        people
    }

    /// The file's people; a file not there yet is made from `origin`, and
    /// only once it is written is the origin told ([`Origin::made`]).
    fn read(&self, origin: &dyn Origin) -> std::io::Result<People> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text).map_err(std::io::Error::other),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let people = origin.first();
                self.write(&people)?;
                origin.made();
                Ok(people)
            }
            Err(e) => Err(e),
        }
    }

    fn write(&self, people: &People) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(people).map_err(std::io::Error::other)?;
        crate::state_fs::devices::write_atomic(&self.path, &bytes)
    }

    /// Read-modify-write under the one lock, written atomically; an
    /// unreadable file is not written over.
    fn modify<R>(&self, origin: &dyn Origin, f: impl FnOnce(&mut People) -> R) -> std::io::Result<R> {
        let mut cache = self.lock();
        let mut people = self.read(origin)?;
        let result = f(&mut people);
        people.version = VERSION;
        self.write(&people)?;
        *cache = Some(Cached {
            stamp: stamp_of(&self.path),
            people: Arc::new(people),
            checked: Instant::now(),
        });
        Ok(result)
    }
}

static STORE: LazyLock<Store> = LazyLock::new(|| Store::at(crate::paths::linggen_home().join("people.json")));

/// Where the first file comes from.
trait Origin {
    /// The people a file not there yet starts with.
    fn first(&self) -> People;
    /// The file is written: what `first` read may be let go.
    fn made(&self) {}
}

/// The device rows, which held each person's level before levels were a
/// person's. Once the file is written the rows' field is cleared, so nothing
/// reads it again.
struct FromDevices;

impl Origin for FromDevices {
    fn first(&self) -> People {
        let devices = crate::state_fs::devices::load_devices();
        first_people(&devices, crate::server::person::mac_account().as_deref())
    }

    fn made(&self) {
        let _ = crate::state_fs::devices::modify_devices(|rows| {
            rows.iter_mut().for_each(|d| d.legacy_permission = None);
        });
    }
}

/// The household's settings as they are.
pub(crate) fn current() -> Arc<People> {
    STORE.current(&FromDevices)
}

/// Change the household's settings, written at once.
pub(crate) fn modify<R>(f: impl FnOnce(&mut People) -> R) -> std::io::Result<R> {
    STORE.modify(&FromDevices, f)
}

/// A person became another (a signed-out phone signed in; a phone paired
/// again as a new row): what was chosen for the phone's person goes to the
/// one it became ([`People::carry`]). Nothing happens unless the person
/// before was a device — the settings of an account stay the account's. The
/// one place this is decided; the sign-in (`rtc::peer::control`) and the
/// pairing (`api::pair`) both ask it.
pub(crate) fn person_changed(before: &Person, after: &Person) {
    if !before.is_device() || before == after {
        return;
    }
    let from = before.wire();
    if let Err(e) = modify(|people| people.carry(&from, after)) {
        tracing::warn!("[people] could not carry a phone's settings to its person: {e}");
    }
}

// ---- turns -------------------------------------------------------------

/// The household person a request is from, when it is one: not the owner's
/// (`user_type` "owner", or a phone that is the owner's), not a room's
/// consumer (the room's config holds it), and not no one.
pub(crate) fn household_person(user_type: &str, user_id: Option<&str>) -> Option<Person> {
    if matches!(user_type, "owner" | "consumer") {
        return None;
    }
    Some(crate::server::person::asking(user_id)).filter(Person::is_another)
}

/// The household person a session is kept for, when it is one: whose
/// `user_id` the session was made under. A session of the owner's or one
/// no one made is not.
pub(crate) fn household_person_of_session(state: &crate::server::ServerState, session_id: &str) -> Option<Person> {
    let meta = state.manager.global_sessions.get_session_meta(session_id).ok().flatten()?;
    Some(Person::from_wire(meta.user_id.as_deref())).filter(Person::is_another)
}

/// Why `person`'s turn must not start now: the owner's models are not
/// shared with them, or the day's tokens are used (`state.token_usage`, the
/// store the room budgets use, keyed by the person as a room's consumer is).
pub(crate) async fn turn_refusal(state: &crate::server::ServerState, person: &Person) -> Option<Refusal> {
    let people = current();
    let used = state.token_usage.lock().await.used_by(&person.wire());
    people.refusal_for(person, used.max(0) as u64)
}

/// Count what `person`'s turn used on this Mac's models against their day.
pub(crate) async fn record_turn(state: &crate::server::ServerState, person: &Person, tokens: u64) {
    if !person.is_another() || tokens == 0 {
        return;
    }
    state.token_usage.lock().await.record_person(&person.wire(), tokens as i64);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::person::stamp_of;
    use crate::state_fs::devices::{AccountRef, DeviceKind};

    fn friend() -> Person {
        Person::for_test(Some("u-friend"))
    }

    fn settings(patch: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        patch.as_object().cloned().unwrap()
    }

    fn row(id: &str, account: Option<&str>, level: Option<PermissionMode>) -> PairedDevice {
        PairedDevice {
            id: id.into(),
            kind: Some(DeviceKind::Phone),
            account: account.map(|a| AccountRef { id: a.into(), email: None, name: None, at: None }),
            legacy_permission: level,
            ..Default::default()
        }
    }

    /// Nobody having set anything: read, every app, none of this Mac's models.
    #[test]
    fn a_person_nobody_set_anything_for_is_at_read_with_every_app_and_no_model() {
        let people = People::default();
        let installed = vec!["music".to_string(), "cfo".to_string()];
        assert_eq!(people.level_of(&friend()), PermissionMode::Read);
        assert_eq!(people.ceiling_of(&friend(), &installed), Ceiling { skills: installed.clone(), models: vec![] });
        assert_eq!(people.refusal_for(&friend(), 0), Some(Refusal::NoModel));
    }

    /// Settings are set by key and read back; a bad one changes nothing.
    #[test]
    fn a_person_is_set_by_key_and_a_bad_setting_changes_nothing() {
        let mut people = People::default();
        let ok = settings(serde_json::json!({
            "level": "edit", "apps": ["music", "music", " "], "models": ["deepseek-flash"], "daily_tokens": 200000
        }));
        people.set(&friend(), &ok).unwrap();
        let got = people.settings_of(&friend());
        assert_eq!(got.level, Some(PermissionMode::Edit));
        assert_eq!(got.apps, Some(vec!["music".to_string()]));
        assert_eq!(got.models, vec!["deepseek-flash".to_string()]);
        assert_eq!(got.daily_tokens, Some(200_000));
        let before = people.clone();
        for bad in [
            serde_json::json!({"level": "admin"}),
            serde_json::json!({"level": "read", "apps": "music"}),
            serde_json::json!({"models": [1]}),
            serde_json::json!({"daily_tokens": -1}),
            serde_json::json!({"colour": "red"}),
        ] {
            assert!(people.set(&friend(), &settings(bad)).is_err());
            assert_eq!(people, before, "a refused setting left a trace");
        }
        assert!(people.set(&Person::test_owner(), &ok).is_err(), "the owner has no row");
        let all = settings(serde_json::json!({"apps": null, "daily_tokens": null}));
        people.set(&friend(), &all).unwrap();
        assert_eq!(people.settings_of(&friend()).apps, None);
        assert_eq!(people.settings_of(&friend()).daily_tokens, None);
    }

    /// An app is the person's when it is on their list and not the owner's
    /// alone; the owner has every one.
    #[test]
    fn an_app_is_asked_in_one_place_and_the_owner_keeps_theirs() {
        let mut people = People::default();
        people.keep_for_owner(vec!["cfo".into()]);
        people.set(&friend(), &settings(serde_json::json!({"apps": ["music", "cfo"]}))).unwrap();
        assert!(people.may_use_app(&friend(), "music"));
        assert!(!people.may_use_app(&friend(), "cfo"), "kept for the owner, though listed");
        assert!(!people.may_use_app(&friend(), "dj"), "not on their list");
        assert!(people.may_use_app(&Person::test_owner(), "cfo"));
        let stranger = Person::for_test(Some("u-stranger"));
        assert!(people.may_use_app(&stranger, "dj"), "no list: every app but the owner's");
        assert!(!people.may_use_app(&stranger, "cfo"));
        let installed = vec!["music".to_string(), "cfo".to_string(), "dj".to_string()];
        assert_eq!(people.ceiling_of(&friend(), &installed).skills, vec!["music".to_string()]);
    }

    /// The turn's refusal: no model shared, then the day's tokens.
    #[test]
    fn a_turn_is_refused_with_no_model_and_over_the_limit() {
        let mut people = People::default();
        assert_eq!(people.refusal_for(&friend(), 0), Some(Refusal::NoModel));
        people
            .set(&friend(), &settings(serde_json::json!({"models": ["m"], "daily_tokens": 100})))
            .unwrap();
        assert_eq!(people.refusal_for(&friend(), 99), None);
        assert_eq!(people.refusal_for(&friend(), 100), Some(Refusal::OverLimit));
        assert_eq!(people.refusal_for(&Person::test_owner(), u64::MAX), None, "the owner is never limited");
        assert_eq!(Refusal::NoModel.status(), "no_model");
        assert_eq!(Refusal::OverLimit.status(), "over_limit");
    }

    /// The first file: a person's level is the lowest any of their rows
    /// held, a row with none holding read; a signed-out phone is its own
    /// person; the owner's rows and rows nobody set are not in it.
    #[test]
    fn the_first_file_gives_a_person_the_lowest_level_their_rows_held() {
        use PermissionMode::{Chat, Edit};
        let mac = Some("u-owner");
        let rows = vec![
            row("a", Some("u-friend"), Some(Edit)),
            row("b", Some("u-friend"), Some(Chat)),
            row("c", Some("u-pal"), Some(Edit)),
            row("d", Some("u-pal"), None),
            row("e", None, Some(Edit)),
            row("f", Some("u-other"), None),
            PairedDevice { kind: Some(DeviceKind::Machine), ..row("g", None, Some(Chat)) },
        ];
        let people = first_people(&rows, mac);
        let level = |p: Person| people.settings_of(&p).level;
        assert_eq!(level(Person::for_test(Some("u-friend"))), Some(Chat));
        assert_eq!(level(Person::for_test(Some("u-pal"))), Some(PermissionMode::Read), "one row unset: read");
        assert_eq!(level(Person::for_test(Some(&stamp_of("e")))), Some(Edit));
        assert_eq!(level(Person::for_test(Some("u-other"))), None);
        assert_eq!(people.people.len(), 3, "the owner's row and unset rows are not kept");
    }

    /// A phone that signs in: its settings become the account's when the
    /// account has none; the account's stand when it has; both said.
    #[test]
    fn a_phone_signing_in_keeps_what_the_owner_chose_or_the_accounts_own() {
        let phone = Person::for_test(Some(&stamp_of("p1")));
        let level = |l: &str| settings(serde_json::json!({"level": l}));
        // The account has none: the phone's become its own.
        let mut people = People::default();
        people.set(&phone, &level("edit")).unwrap();
        assert_eq!(people.carry(&phone.wire(), &friend()), Some(Arrival::Moved));
        assert_eq!(people.settings_of(&friend()).level, Some(PermissionMode::Edit));
        assert_eq!(people.settings_of(&friend()).arrival, Some(Arrival::Moved));
        assert_eq!(people.settings_of(&phone), PersonSettings::default(), "the phone's are gone");
        // The account has its own: they stand, the phone's are dropped.
        let mut people = People::default();
        people.set(&phone, &level("edit")).unwrap();
        people.set(&friend(), &level("chat")).unwrap();
        assert_eq!(people.carry(&phone.wire(), &friend()), Some(Arrival::Kept));
        assert_eq!(people.settings_of(&friend()).level, Some(PermissionMode::Chat));
        assert_eq!(people.settings_of(&friend()).arrival, Some(Arrival::Kept));
        assert_eq!(people.settings_of(&phone), PersonSettings::default());
        // Nothing was chosen for the phone: nothing to say.
        assert_eq!(People::default().carry(&phone.wire(), &friend()), None);
    }

    /// A phone paired again is a new row of the same device: its settings
    /// go to its new stamp. A phone that became the owner's has no row.
    #[test]
    fn a_phone_paired_again_keeps_its_settings_and_a_phone_that_became_the_owner_has_none() {
        let old = Person::for_test(Some(&stamp_of("p1")));
        let new = Person::for_test(Some(&stamp_of("p2")));
        let mut people = People::default();
        people.set(&old, &settings(serde_json::json!({"level": "edit"}))).unwrap();
        assert_eq!(people.carry(&old.wire(), &new), None);
        assert_eq!(people.settings_of(&new).level, Some(PermissionMode::Edit));
        assert_eq!(people.settings_of(&old), PersonSettings::default());
        assert_eq!(people.carry(&new.wire(), &Person::test_owner()), None);
        assert_eq!(people.people.len(), 0, "the owner has no row");
    }

    /// A file's origin for a test: the rows it is given, and whether the
    /// rows were let go once it was written.
    struct Rows(Vec<PairedDevice>, std::cell::Cell<bool>);

    impl Origin for Rows {
        fn first(&self) -> People {
            first_people(&self.0, Some("u-owner"))
        }
        fn made(&self) {
            self.1.set(true);
        }
    }

    struct Nothing;

    impl Origin for Nothing {
        fn first(&self) -> People {
            People::default()
        }
    }

    /// The file is made from the rows when it is not there, written before
    /// the rows are let go, and read back; a file that does not read is not
    /// written over.
    #[test]
    fn the_file_is_made_once_kept_and_never_written_over_unread() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("people.json");
        let store = Store::at(path.clone());
        let rows = Rows(vec![row("a", Some("u-friend"), Some(PermissionMode::Chat))], Default::default());
        let people = store.current(&rows);
        assert_eq!(people.level_of(&friend()), PermissionMode::Chat);
        assert!(path.exists() && rows.1.get(), "written, then the rows let go");
        store
            .modify(&Nothing, |p| p.set(&friend(), &settings(serde_json::json!({"level": "edit"}))))
            .unwrap()
            .unwrap();
        let again = Store::at(path.clone()).current(&Nothing);
        assert_eq!(again.level_of(&friend()), PermissionMode::Edit, "the first file was made once");
        std::fs::write(&path, "{ not json").unwrap();
        let broken = Store::at(path.clone());
        assert_eq!(broken.current(&Nothing).level_of(&friend()), PermissionMode::Read);
        assert!(broken.modify(&Nothing, |_| ()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
        // A file that cannot be written leaves the rows as they are.
        std::fs::write(dir.path().join("x"), "a file, not a folder").unwrap();
        let unwritable = Store::at(dir.path().join("x").join("people.json"));
        let kept = Rows(vec![row("a", Some("u-friend"), Some(PermissionMode::Chat))], Default::default());
        let _ = unwritable.current(&kept);
        assert!(!kept.1.get(), "the rows were let go before the file was written");
    }
}

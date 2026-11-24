//! A ticked family: one ordinary row that follows its family, or is pinned.
//!
//! The pieces, each deciding one thing:
//! - `binding` — the only writer of a row's [`FamilyBinding`] and of the
//!   starter record; every change of a ticked row goes through it.
//! - `evidence` — which model ids a provider lists for this endpoint and key.
//! - `follow` — which following rows move, and the go-back of a refused move.
//! - `act` — what a person's tick, untick, pin, follow, choose, acknowledge
//!   and go-back do.
//! - `view` — what the settings list says about each family.
//!
//! [`Families`] is the one object the server holds. Its methods that change a
//! config take `&mut Config` and run under the server's settings write lock;
//! the one that asks a provider (`gather`) runs outside it.

mod act;
mod binding;
mod evidence;
mod follow;
mod view;

pub use act::{Action, Refusal};
pub use binding::{FamilyBinding, FamilyStarts, LastMove, Mode, MoveKind};
pub use view::FamiliesView;
use binding::keep_bindings;

use super::curated::{self, Bundle, Place};
use super::family_catalog::{family_current, Access, AccessEvidence, Connection, FamilyCurrent};
use crate::config::{Config, ModelConfig};
use crate::credentials::{self, Credentials};
use chrono::NaiveDate;
use evidence::{Evidence, Listing};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

pub struct Families {
    evidence: Evidence,
    /// A fixed day for the system tests; the clock otherwise.
    today: Option<NaiveDate>,
    /// A fixed list for the unit tests; the one in force otherwise.
    fixed_catalog: Option<Arc<Bundle>>,
    /// Rows whose first call on a just-moved model was refused, waiting for
    /// the go-back under the settings write lock.
    refused: Mutex<BTreeSet<(String, String)>>,
    /// Rows moved in this run whose new model has not answered a call yet:
    /// only their first call can send them back. In memory on purpose — a
    /// restart disarms them, and a stray refusal weeks later never does.
    armed: Mutex<BTreeSet<String>>,
    wake: tokio::sync::Notify,
}

/// System tests (a `test-support` build only): provider lists and the day
/// come from this file in the home, `{"today": "YYYY-MM-DD", "lists":
/// {"<preset key>": ["<model id>", …]}}`, read at each ask.
#[cfg(feature = "test-support")]
const TEST_FIXTURE: &str = "test-families.json";

impl Default for Families {
    fn default() -> Self { Self::new() }
}

impl Families {
    pub fn new() -> Self {
        #[cfg(feature = "test-support")]
        {
            let path = crate::paths::linggen_home().join(TEST_FIXTURE);
            if path.exists() {
                let today = std::fs::read_to_string(&path).ok()
                    .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                    .and_then(|v| v.get("today")?.as_str()?.parse().ok());
                return Self::with(Evidence::from_fixture(path), today);
            }
            // A test build without a fixture reaches no provider.
            return Self::with(Evidence::unreachable(), None);
        }
        #[cfg(not(feature = "test-support"))]
        Self::with(Evidence::http(), None)
    }

    pub(crate) fn with(evidence: Evidence, today: Option<NaiveDate>) -> Self {
        Self { evidence, today, fixed_catalog: None, refused: Default::default(), armed: Default::default(), wake: Default::default() }
    }

    /// The list this decision is made from: read once, so one decision never mixes two.
    fn catalog(&self) -> Arc<Bundle> {
        self.fixed_catalog.clone().unwrap_or_else(curated::current)
    }

    fn today(&self) -> NaiveDate { self.today.unwrap_or_else(|| chrono::Utc::now().date_naive()) }

    /// Ask the named endpoint for a settings action, or all filed key
    /// endpoints for discovery. Runs outside the settings write lock.
    pub async fn gather(&self, config: &Config, creds: &Credentials, endpoint: Option<(&str, &str)>) {
        // A key is asked only at the address it is filed under: an endpoint is
        // found by a row's or a credential's own address, never by a list's.
        let list = self.catalog();
        let mut asks: Vec<(Place<'_>, String)> = Vec::new();
        let named = endpoint.and_then(|(name, url)| Endpoint::of(&list, name, url, creds));
        let rows = config.models.iter().filter_map(|row| Endpoint::of_row(&list, row, creds));
        let filed = creds.endpoints.iter().filter_map(|e| Endpoint::of(&list, &e.provider, &e.url, creds));
        for e in named.into_iter().chain(rows.chain(filed).filter(|_| endpoint.is_none())) {
            let address = e.place.base;
            if let Some(key) = e.key.filter(|_| !asks.iter().any(|(p, _)| p.endpoint_key == e.place.endpoint_key && p.is_at(address))) {
                asks.push((e.place, key));
            }
        }
        futures_util::future::join_all(asks.iter().map(|(place, key)| async move {
            self.evidence.gather(place, key).await;
        })).await;
    }

    /// Move every following row the selector says should move, with its
    /// notice. True when the config changed.
    pub fn follow(&self, config: &mut Config, creds: &Credentials) -> bool {
        follow::follow_all(self, config, creds)
    }

    /// Carry out a person's action on one family of an endpoint. Gather the
    /// provider's list first (`gather`) when [`Action::needs_list`].
    pub fn act(&self, config: &mut Config, creds: &Credentials, endpoint: (&str, &str), family: &str, action: Action) -> Result<bool, Refusal> {
        act::run(self, config, creds, endpoint, family, action)
    }

    /// The first key of an endpoint: tick its starter families, once.
    pub fn start_endpoint(&self, config: &mut Config, creds: &Credentials, endpoint: (&str, &str)) -> bool {
        follow::start(self, config, creds, endpoint)
    }

    /// A full config posted by the legacy settings route, made against an
    /// older read, must not undo a move or set a binding: call this under
    /// the write lock with the config in force.
    pub fn keep_bindings(next: &mut Config, current: &Config) {
        keep_bindings(next, current)
    }

    /// A call on `model_id` succeeded: its move, if it had one, is proven,
    /// and no later refusal sends the row back. Cheap; called on every answer.
    pub fn model_answered(&self, model_id: &str) {
        let mut armed = self.armed.lock().unwrap_or_else(|e| e.into_inner());
        if !armed.is_empty() {
            armed.remove(model_id);
        }
    }

    /// Whether a refusal on `model_id` could send it back: it moved in this
    /// run and has not answered since.
    pub fn is_armed(&self, model_id: &str) -> bool {
        self.armed.lock().unwrap_or_else(|e| e.into_inner()).contains(model_id)
    }

    fn arm(&self, row_id: &str) {
        self.armed.lock().unwrap_or_else(|e| e.into_inner()).insert(row_id.to_string());
    }

    fn disarm(&self, row_id: &str) {
        self.armed.lock().unwrap_or_else(|e| e.into_inner()).remove(row_id);
    }

    /// A call on `model_id` was refused. When that is the first call on a
    /// followed row's newer model, queue the go-back. Never blocks; nothing
    /// is written here.
    pub fn model_refused(&self, config: &Config, model_id: &str, err: &anyhow::Error) {
        if self.is_armed(model_id) && follow::wants_go_back(config, model_id, err) {
            let wire = config.models.iter().find(|row| row.id == model_id).map(|row| row.model.clone()).unwrap_or_default();
            self.refused.lock().unwrap_or_else(|e| e.into_inner()).insert((model_id.to_string(), wire));
            self.wake.notify_one();
        }
    }

    /// Wait until a refusal is queued.
    pub async fn refusal_queued(&self) { self.wake.notified().await }

    /// Go back on every queued refusal. True when the config changed.
    pub fn go_back_refused(&self, config: &mut Config) -> bool {
        let queued = std::mem::take(&mut *self.refused.lock().unwrap_or_else(|e| e.into_inner()));
        follow::go_back(self, config, queued)
    }

    /// What the settings list says about an endpoint's families. Reads held
    /// evidence only; never asks a provider.
    pub fn view(&self, config: &Config, creds: &Credentials, endpoint: (&str, &str)) -> Option<FamiliesView> {
        view::of(self, config, creds, endpoint)
    }
}

/// An endpoint as the families see it: where the list places it, and the key
/// a row there would call with (one of the key's sources, never shown).
struct Endpoint<'a> {
    place: Place<'a>,
    probe: ModelConfig,
    key: Option<String>,
}

impl<'a> Endpoint<'a> {
    fn of(list: &'a Bundle, name: &str, url: &str, creds: &Credentials) -> Option<Self> {
        let probe: ModelConfig = serde_json::from_value(serde_json::json!({
            "id": "endpoint-probe", "provider": name, "url": url, "model": "-",
        })).ok()?;
        Self::of_row(list, &probe, creds)
    }

    fn of_row(list: &'a Bundle, row: &ModelConfig, creds: &Credentials) -> Option<Self> {
        let place = list.place_of(row)?;
        Some(Self { place, probe: row.clone(), key: credentials::resolve_api_key(row, creds) })
    }

    /// Ask the selector about `family` for this endpoint. `listing` is the
    /// held evidence; it counts only for this endpoint's current key.
    fn ask(&self, list: &'a Bundle, family: &str, current: Option<&'a str>, listing: Option<&Listing>, today: NaiveDate) -> FamilyCurrent<'a> {
        let evidence = listing.map(|l| AccessEvidence {
            connection: Connection { endpoint: &self.probe, credential_revision: l.revision },
            usable: &l.usable,
        });
        let access = match (&self.key, evidence.as_ref()) {
            (_, Some(evidence)) => Access::Listed(evidence),
            (Some(_), None) => Access::Unchecked,
            (None, None) => Access::NoKey,
        };
        self.select(list, family, current, access, today)
    }

    fn select(&self, list: &'a Bundle, family: &str, current: Option<&'a str>, access: Access<'_>, today: NaiveDate) -> FamilyCurrent<'a> {
        let credential_revision = self.key.as_deref().map_or(0, evidence::revision_of);
        family_current(list, Connection { endpoint: &self.probe, credential_revision }, family, current, access, today)
    }

    fn read_result(&self, families: &Families) -> Option<Option<Arc<Listing>>> {
        families.evidence.result(&self.place, self.key.as_deref()?)
    }

    fn listing(&self, families: &Families) -> Option<Arc<Listing>> {
        families.evidence.held(&self.place, self.key.as_deref()?)
    }
}

#[cfg(test)]
mod tests;

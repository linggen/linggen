//! Settings' provider groups: a projection of the existing rows and endpoint keys.

use super::{ModelManager, NotReady};
use crate::config::providers::Way;
use super::curated::{self, Bundle, Edition};
use super::ModelCatalog;
use crate::config::{Config, ModelConfig, SignIn};
use crate::credentials::{self, Credentials};
use crate::provider::kinds::{kind_of, Kind};
use crate::{live, runtime};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Serialize)]
pub struct ProviderView {
    pub id: String,
    label: String,
    pub way: &'static str,
    pub provider: String,
    pub url: String,
    /// The built-in list owns this address: the page shows no address field.
    listed: bool,
    connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    fix: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<KeyState>,
    /// A connection the list names only in the other edition: the person's own,
    /// kept as it is and not offered to anyone new.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    retained: bool,
    pub models: Vec<ProviderModel>,
    used_by: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    disk_gb: Option<u64>,
    /// The catalog's families at this endpoint (`families::Families::view`),
    /// filled by the route that has them; absent where there are none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub families: Option<super::families::FamiliesView>,
    /// This month's estimated spend and the cap, for a key provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    spend: Option<super::spend_cap::SpendView>,
}

#[derive(Serialize)]
struct KeyState {
    set: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    conflict: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    can_keep_model_key: bool,
}

#[derive(Serialize)]
pub struct ProviderModel {
    pub id: String,
    model: String,
    kind: &'static str,
    ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    why: Option<NotReady>,
    pub builtin: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    first: bool,
    /// What it costs per million tokens, when a price is known (a custom model's: asked of the site).
    #[serde(skip_serializing_if = "Option::is_none")]
    price: Option<super::families::PriceLine>,
}

/// Settings identity only. Credential lookup still uses the filed provider name.
pub fn provider_id(sign_in: SignIn, name: &str, url: &str) -> String {
    let class = match sign_in {
        SignIn::Key => "key",
        SignIn::ChatGpt => "chatgpt",
        SignIn::Claude => "claude",
        SignIn::LinggenAccount => "account",
        SignIn::Unknown => "unknown",
    };
    // The one rule: a key card is its address. Names filed under it do not split it.
    // Local lanes (ollama) and rows with no address keep their name.
    let by_address = sign_in == SignIn::Key && !credentials::endpoint_url(url).is_empty()
        && !name.trim().eq_ignore_ascii_case("ollama");
    let name = if by_address {
        String::new()
    } else {
        name.trim().to_ascii_lowercase()
    };
    let parts = (class, name, credentials::endpoint_url(url).to_ascii_lowercase());
    format!("{:x}", Sha256::digest(serde_json::to_vec(&parts).expect("strings serialize")))
}

/// The filed endpoint settings should edit, separately from the card's label.
/// Prefer the endpoint with a key used by a model, then the first filed key.
pub fn filed_endpoint<'a>(config: &'a Config, creds: &'a Credentials, id: &str) -> Option<(&'a str, &'a str)> {
    let rows = || config.models.iter().filter(|row| editable_row(row, id));
    let keyed_row = rows().find(|row| uses_endpoint_key(row, creds));
    let row_pair = |r: &'a ModelConfig| (&*r.provider, &*r.url);
    keyed_row.map(row_pair)
        .or_else(|| group_keys(creds, id).next().map(|e| (&*e.provider, &*e.url)))
        .or_else(|| rows().next().map(row_pair))
}

pub fn editable_row(row: &ModelConfig, id: &str) -> bool {
    !row.is_builtin && row.sign_in() == SignIn::Key && provider_id(row.sign_in(), &row.provider, &row.url) == id
}

pub fn group_keys<'a, 'id>(creds: &'a Credentials, id: &'id str) -> impl Iterator<Item = &'a credentials::EndpointKey> + use<'a, 'id> {
    creds.endpoints.iter().filter(move |e| provider_id(SignIn::Key, &e.provider, &e.url) == id)
}

fn uses_endpoint_key(row: &ModelConfig, creds: &Credentials) -> bool {
    row.api_key.as_deref().is_none_or(str::is_empty) && creds.model_override(&row.id).is_none()
        && creds.endpoint_key(&row.provider, &row.url).is_some()
}

/// Compute the cleanup on a copy. Refuse if no model uses an endpoint key,
/// or if removing any different key would change a model's or live voice's key.
/// The view and the owner-only write use this same check; no keys leave it.
pub fn keep_model_key(config: &Config, creds: &Credentials, id: &str) -> Option<Credentials> {
    let row = config.models.iter().find(|r| editable_row(r, id) && uses_endpoint_key(r, creds))?;
    let key = creds.endpoint_key(&row.provider, &row.url)?;
    let mut next = creds.clone();
    next.endpoints.retain(|e| provider_id(SignIn::Key, &e.provider, &e.url) != id
        || e.api_key.as_deref().is_none_or(|k| k.is_empty() || k == key));
    if next.endpoints.len() == creds.endpoints.len()
        || config.models.iter().any(|row| credentials::resolve_api_key(row, creds) != credentials::resolve_api_key(row, &next))
        || live::PROVIDERS.iter().any(|p| live::stored_key(p, &config.models, creds) != live::stored_key(p, &config.models, &next))
    {
        return None;
    }
    Some(next)
}

/// Display an address, not credentials embedded in a pasted URL.
fn shown_url(url: &str) -> String {
    if url.is_empty() {
        return String::new();
    }
    let Ok(mut parsed) = url::Url::parse(url.trim()) else { return String::new() };
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.set_query(None);
    parsed.set_fragment(None);
    parsed.to_string()
}

/// Source shown for a configured model, shared by provider cards and `/api/uses`.
pub fn provider_source(sign_in: SignIn, provider: &str, url: &str) -> (String, Way) {
    provider_source_in(&curated::current(), sign_in, provider, url)
}

fn provider_source_in(list: &Bundle, sign_in: SignIn, provider: &str, url: &str) -> (String, Way) {
    match sign_in {
        SignIn::LinggenAccount => ("Linggen Cloud".into(), Way::Account),
        SignIn::ChatGpt => ("ChatGPT 订阅".into(), Way::Subscription),
        SignIn::Claude => ("Claude 订阅".into(), Way::Subscription),
        SignIn::Key | SignIn::Unknown => {
            let way = if provider.eq_ignore_ascii_case("ollama") { Way::Local } else { Way::Key };
            (list.label_for(provider, url), way)
        }
    }
}

/// Whether the built-in list owns this key address: its address is never edited.
pub fn is_listed_address(url: &str) -> bool {
    curated::current().place_at(url).is_some_and(|p| p.provider.sign_in == Way::Key)
}

impl ProviderView {
    /// An editable endpoint proposed by the owner, not yet filed on disk.
    pub fn draft(catalog: &ModelCatalog, provider: &str, url: &str) -> Self {
        let mut group = Self::new(&curated::current(), catalog.offered(), SignIn::Key, provider, url);
        if group.key.is_some() {
            group.fix = Some("add_key");
        }
        group
    }

    fn new(list: &Bundle, offered: Edition, sign_in: SignIn, provider: &str, url: &str) -> Self {
        let (label, way) = provider_source_in(list, sign_in, provider, url);
        // A card at a listed address is filed under the list's name and address.
        let place = (way == Way::Key).then(|| list.place_at(url).filter(|p| p.provider.sign_in == Way::Key)).flatten();
        let listed = place.is_some();
        let retained = way == Way::Key && list.kept_out_of(offered, provider, url);
        let (provider, url) = place.map_or((provider, url), |p| (p.provider.name.as_str(), p.base));
        Self {
            id: provider_id(sign_in, provider, url), label, way: way.as_str(),
            provider: provider.into(), url: shown_url(url), listed, connected: false, fix: None, retained,
            key: (way == Way::Key).then_some(KeyState { set: false, conflict: false, can_keep_model_key: false }),
            models: Vec::new(), used_by: Vec::new(), disk_gb: None, families: None, spend: None,
        }
    }

    fn add(&mut self, row: ProviderModel) {
        self.connected |= row.ready;
        if !self.used_by.contains(&row.kind) {
            self.used_by.push(row.kind);
        }
        self.models.push(row);
    }

    fn mark_key(&mut self, set: bool) {
        if let Some(key) = &mut self.key {
            key.set |= set;
        }
    }
}

impl ModelManager {
    pub fn providers_view(&self, config: &Config, creds: &Credentials) -> Vec<ProviderView> {
        let list = curated::current();
        let offered = config.model_catalog.offered();
        let mut groups = BTreeMap::new();
        // A user row may shadow the cloud model without removing the sign-in.
        let cloud_url = super::catalog::linggen_cloud_url();
        let mut cloud = ProviderView::new(&list, offered, SignIn::LinggenAccount, super::catalog::LINGGEN_CLOUD_PROVIDER, &cloud_url);
        cloud.connected = crate::account::resolve_token()
            .is_some_and(|(token, _)| !crate::account::token_rejected(&token));
        cloud.fix = (!cloud.connected).then_some("sign_in");
        for use_ in cloud_uses(&list, offered) {
            cloud.add(use_);
        }
        groups.insert(cloud.id.clone(), cloud);
        let rows = self.list_models();
        for row in rows.iter().copied().filter(|row| row.is_builtin).chain(config.chat_models()) {
            let kind = kind_of(row);
            if kind != Kind::Chat {
                continue;
            }
            let sign_in = row.sign_in();
            let id = provider_id(sign_in, row.provider_name(), &row.url);
            let group = groups.entry(id).or_insert_with(|| ProviderView::new(&list, offered, sign_in, row.provider_name(), &row.url));
            if group.models.iter().any(|m| m.id == row.id) {
                continue;
            }
            group.mark_key(credentials::resolve_api_key(row, creds).is_some());
            let why = if rows.iter().any(|m| m.id == row.id && m.is_builtin == row.is_builtin) {
                self.readiness(&row.id).and_then(|r| r.reason)
            } else {
                Some(NotReady::ProviderFailing)
            };
            group.add(ProviderModel { id: row.id.clone(), model: row.model.clone(), kind: kind.as_str(), ready: why.is_none(), why, builtin: row.is_builtin, first: false, price: super::price_of(row).map(super::families::PriceLine::of) });
        }
        for endpoint in &creds.endpoints {
            // A credential has no sign-in field; only the key group can use it.
            let id = provider_id(SignIn::Key, &endpoint.provider, &endpoint.url);
            let group = groups.entry(id).or_insert_with(|| ProviderView::new(&list, offered, SignIn::Key, &endpoint.provider, &endpoint.url));
            let set = creds.endpoint_key(&endpoint.provider, &endpoint.url).is_some();
            group.mark_key(set);
            group.connected |= set && group.models.is_empty();
        }
        for voice in live::offered_to_pages().into_iter() {
            let ready = live::key_ready(voice, &config.models, creds);
            // The line is shown where the list names the voice; a voice it does not name has none.
            let Some((place, _)) = list.live_place(voice.id) else { continue };
            let (name, base) = (place.provider.name.as_str(), place.base);
            let id = provider_id(SignIn::Key, name, base);
            let group = groups.entry(id).or_insert_with(|| ProviderView::new(&list, offered, SignIn::Key, name, base));
            // A legacy live row can supply its key, but only the listed provider owns the line.
            group.mark_key(ready);
            group.add(ProviderModel {
                id: format!("live:{}", voice.id), model: voice.model.into(), kind: Kind::LiveVoice.as_str(),
                ready, why: (!ready).then_some(NotReady::NoKey), builtin: true, first: false, price: None,
            });
        }
        let mut providers: Vec<_> = groups.into_values().collect();
        for group in &mut providers {
            if let Some(key) = &mut group.key {
                let mut keys = group_keys(creds, &group.id).filter_map(|e| e.api_key.as_deref()).filter(|k| !k.is_empty());
                key.conflict = keys.next().is_some_and(|first| keys.any(|k| k != first));
                key.can_keep_model_key = key.conflict && keep_model_key(config, creds, &group.id).is_some();
            }
            group.spend = (group.way == "key").then(|| super::spend_cap::view(&group.id));
            group.models.sort_by(|a, b| a.id.cmp(&b.id));
            group.used_by.sort_unstable();
            group.fix = match group.way {
                "key" if group.key.as_ref().is_some_and(|k| !k.set) && !group.connected => Some("add_key"),
                "account" | "subscription" if group.fix.is_some() || group.models.iter().any(|m| matches!(m.why, Some(NotReady::NotSignedIn | NotReady::SignInExpired | NotReady::ProviderSignIn))) => Some("sign_in"),
                _ => None,
            };
        }
        providers.extend(local_lanes(offered));
        providers.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.id.cmp(&b.id)));
        providers
    }
}

/// Linggen Cloud's voice uses the list names — live voice and read-aloud — as
/// lines on its card, ready when a subscription is known (no network). Only a
/// use the list names AND the engine has a wire for is here (`curated`).
fn cloud_uses(list: &Bundle, offered: Edition) -> Vec<ProviderModel> {
    list.cloud_uses(offered).map(|entry| {
        let (kind, ready) = match entry.kind {
            curated::EntryType::Speech => (Kind::Speak, crate::account::cloud_voice::speech_usable()),
            _ => (Kind::LiveVoice, crate::account::cloud_voice::live_usable()),
        };
        let id = format!("{}:cloud:{}", kind.as_str(), entry.key);
        // Signed in without a subscription: not ready, and no reason a sign-in would fix.
        let why = (!ready && crate::account::resolve_token().is_none()).then_some(NotReady::NotSignedIn);
        ProviderModel { id, model: entry.model.clone(), kind: kind.as_str(), ready, why, builtin: true, first: false, price: None }
    }).collect()
}

/// Whether the list names read-aloud under Linggen Cloud: the picker shows
/// "Linggen Cloud · Serena" only then (`api/uses`, `tts::Source`).
pub fn cloud_speech_listed() -> bool {
    curated::current().lists_cloud_speech()
}

/// What the page offers to add, as the curated list names it, in its order:
/// a provider or a model added to the list shows here with no code change.
pub fn presets(catalog: &ModelCatalog) -> Vec<serde_json::Value> {
    let list = curated::current();
    list.offered(catalog.offered()).map(|p| serde_json::json!({
        "name": p.provider.name, "label": p.provider.label, "url": p.base, "way": p.provider.sign_in.as_str(),
        "live": p.offered_live_models(), "unverified": p.unverified(),
    })).collect()
}

/// The provider name and address a person's pick (a label, an address or a name) stands for.
pub fn preset_named(catalog: &ModelCatalog, selector: &str) -> Option<(String, String, Way)> {
    let list = curated::current();
    let offered = catalog.offered();
    let place = list.offered(offered).find(|p| p.provider.label == selector || p.base == selector)
        .or_else(|| list.offered(offered).find(|p| p.provider.name == selector))?;
    Some((place.provider.name.clone(), place.base.to_string(), place.provider.sign_in))
}

/// Where a live voice's card sits when it has no row or key yet: the list names it.
pub fn live_card_at(id: &str) -> Option<(String, String)> {
    let list = curated::current();
    let (place, _) = live::offered_to_pages().and_then(|voice| list.live_place(voice.id))?;
    (provider_id(SignIn::Key, &place.provider.name, place.base) == id)
        .then(|| (place.provider.name.clone(), place.base.to_string()))
}

fn local_lanes(offered: Edition) -> impl Iterator<Item = ProviderView> {
    use runtime::{HUB_MODELS, PICTURES_LANE, PICTURE_MODEL, STT_MODEL, TTS_MODEL, VOICE_LANE};
    [
        (Kind::Speak, TTS_MODEL, VOICE_LANE, runtime::voice_ready()),
        (Kind::Hear, STT_MODEL, VOICE_LANE, runtime::hearing_ready()),
        (Kind::Draw, PICTURE_MODEL, PICTURES_LANE, runtime::pictures_ready()),
    ].into_iter()
    // A Mac hears with Apple's own recognition: no model of ours to show (`runtime::hearing_by_apple`).
    .filter(|(kind, ..)| !(*kind == Kind::Hear && runtime::hearing_by_apple()))
    .map(move |(kind, model, lane, on_disk)| {
        let ready = on_disk && runtime::lane_verdict(&lane).is_ok();
        let hub = HUB_MODELS.iter().find(|m| m.repo == model).expect("local model is declared");
        let mut group = ProviderView::new(&curated::current(), offered, SignIn::Key, "ollama", "");
        group.id = provider_id(SignIn::Key, "ollama", kind.as_str());
        group.label = "This Mac".into();
        group.fix = (!ready).then(|| if runtime::env_bin(hub.venv, "python3").exists() { "download" } else { "install" });
        group.disk_gb = Some(hub.need_gb);
        group.add(ProviderModel { id: kind.as_str().into(), model: model.into(), kind: kind.as_str(), ready, why: None, builtin: true, first: false, price: None });
        group
    })
}

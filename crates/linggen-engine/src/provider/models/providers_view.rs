//! Settings' provider groups: a projection of the existing rows and endpoint keys.

use super::{presets, ModelManager, NotReady};
use crate::config::providers::Way;
use crate::config::{Config, ModelConfig, SignIn};
use crate::credentials::{self, Credentials};
use crate::provider::kinds::{kind_of, Kind};
use crate::live;
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
    /// A preset owns this address: the page shows no address field.
    listed: bool,
    /// The API style a key provider speaks (`presets::ApiStyle`).
    #[serde(skip_serializing_if = "Option::is_none")]
    style: Option<&'static str>,
    connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    fix: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<KeyState>,
    pub models: Vec<ProviderModel>,
    used_by: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    disk_gb: Option<u64>,
    /// Linggen Cloud's chat models to tick (`cloud_models`); absent elsewhere.
    #[serde(skip_serializing_if = "Option::is_none")]
    cloud: Option<Vec<CloudLine>>,
}

/// One Linggen Cloud model as its row shows it: a checkbox.
#[derive(Serialize)]
struct CloudLine {
    model: String,
    label: String,
    /// The row that runs it, when ticked.
    #[serde(skip_serializing_if = "Option::is_none")]
    row: Option<String>,
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
    /// The name a page shows: Linggen Cloud's label, else the id.
    name: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    first: bool,
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
    match sign_in {
        SignIn::LinggenAccount => ("Linggen Cloud".into(), Way::Account),
        SignIn::ChatGpt => ("ChatGPT 订阅".into(), Way::Subscription),
        SignIn::Claude => ("Claude 订阅".into(), Way::Subscription),
        SignIn::Key | SignIn::Unknown => {
            // Ollama is one provider whatever its address: its label never becomes the host.
            if provider.eq_ignore_ascii_case("ollama") { return ("Ollama".into(), Way::Local); }
            (presets::label_for(provider, url), Way::Key)
        }
    }
}

/// Whether a preset owns this key address: its address is never edited.
pub fn is_listed_address(url: &str) -> bool {
    presets::at(url).is_some()
}

impl ProviderView {
    /// An editable endpoint proposed by the owner, not yet filed on disk.
    pub fn draft(provider: &str, url: &str) -> Self {
        let mut group = Self::new(SignIn::Key, provider, url);
        if group.key.is_some() {
            group.fix = Some("add_key");
        }
        group
    }

    fn new(sign_in: SignIn, provider: &str, url: &str) -> Self {
        let (label, way) = provider_source(sign_in, provider, url);
        // A card at a preset's address is filed under the preset's name and address.
        let preset = (way == Way::Key).then(|| presets::at(url)).flatten();
        let listed = preset.is_some();
        let (provider, url) = preset.map_or((provider, url), |p| (p.name, p.url));
        let style = (way == Way::Key).then(|| {
            let claude = provider.trim().eq_ignore_ascii_case("anthropic");
            if claude { presets::ApiStyle::Claude } else { presets::ApiStyle::OpenAiCompatible }.as_str()
        });
        Self {
            id: provider_id(sign_in, provider, url), label, way: way.as_str(),
            provider: provider.into(), url: shown_url(url), listed, style, connected: false, fix: None,
            key: (way == Way::Key).then_some(KeyState { set: false, conflict: false, can_keep_model_key: false }),
            models: Vec::new(), used_by: Vec::new(), disk_gb: None, cloud: None,
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
        let mut groups = BTreeMap::new();
        // A user row may shadow the cloud model without removing the sign-in.
        let cloud_url = super::catalog::linggen_cloud_url();
        let mut cloud = ProviderView::new(SignIn::LinggenAccount, super::catalog::LINGGEN_CLOUD_PROVIDER, &cloud_url);
        cloud.connected = crate::account::resolve_token()
            .is_some_and(|(token, _)| !crate::account::token_rejected(&token));
        cloud.fix = (!cloud.connected).then_some("sign_in");
        for use_ in cloud_uses() {
            cloud.add(use_);
        }
        cloud.cloud = Some(cloud_lines(config));
        groups.insert(cloud.id.clone(), cloud);
        let rows = self.list_models();
        for row in rows.iter().copied().filter(|row| row.is_builtin).chain(config.chat_models()) {
            let kind = kind_of(row);
            if kind != Kind::Chat {
                continue;
            }
            let sign_in = row.sign_in();
            let id = provider_id(sign_in, row.provider_name(), &row.url);
            let group = groups.entry(id).or_insert_with(|| ProviderView::new(sign_in, row.provider_name(), &row.url));
            if group.models.iter().any(|m| m.id == row.id) {
                continue;
            }
            group.mark_key(credentials::resolve_api_key(row, creds).is_some());
            let why = if rows.iter().any(|m| m.id == row.id && m.is_builtin == row.is_builtin) {
                self.readiness(&row.id).and_then(|r| r.reason)
            } else {
                Some(NotReady::ProviderFailing)
            };
            group.add(ProviderModel { id: row.id.clone(), model: row.model.clone(), kind: kind.as_str(), ready: why.is_none(), why, builtin: row.is_builtin, name: self.display_name(row), first: false });
        }
        for endpoint in &creds.endpoints {
            // A credential has no sign-in field; only the key group can use it.
            let id = provider_id(SignIn::Key, &endpoint.provider, &endpoint.url);
            let group = groups.entry(id).or_insert_with(|| ProviderView::new(SignIn::Key, &endpoint.provider, &endpoint.url));
            let set = creds.endpoint_key(&endpoint.provider, &endpoint.url).is_some();
            group.mark_key(set);
            group.connected |= set && group.models.is_empty();
        }
        for voice in live::offered_to_pages().into_iter() {
            let ready = live::key_ready(voice, &config.models, creds);
            // The line is shown on the preset at the voice's host; a host with no preset has none.
            let Some(preset) = live_preset(voice) else { continue };
            let id = provider_id(SignIn::Key, preset.name, preset.url);
            let group = groups.entry(id).or_insert_with(|| ProviderView::new(SignIn::Key, preset.name, preset.url));
            // A legacy live row can supply its key, but only the preset owns the line.
            group.mark_key(ready);
            group.add(ProviderModel {
                id: format!("live:{}", voice.id), model: voice.model.into(), kind: Kind::LiveVoice.as_str(),
                ready, why: (!ready).then_some(NotReady::NoKey), builtin: true, name: voice.model.into(), first: false,
            });
        }
        let mut providers: Vec<_> = groups.into_values().collect();
        for group in &mut providers {
            if let Some(key) = &mut group.key {
                let mut keys = group_keys(creds, &group.id).filter_map(|e| e.api_key.as_deref()).filter(|k| !k.is_empty());
                key.conflict = keys.next().is_some_and(|first| keys.any(|k| k != first));
                key.can_keep_model_key = key.conflict && keep_model_key(config, creds, &group.id).is_some();
            }
            group.models.sort_by(|a, b| a.id.cmp(&b.id));
            group.used_by.sort_unstable();
            group.fix = match group.way {
                "key" if group.key.as_ref().is_some_and(|k| !k.set) && !group.connected => Some("add_key"),
                "account" | "subscription" if group.fix.is_some() || group.models.iter().any(|m| matches!(m.why, Some(NotReady::NotSignedIn | NotReady::SignInExpired | NotReady::ProviderSignIn))) => Some("sign_in"),
                _ => None,
            };
        }
        providers.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.id.cmp(&b.id)));
        providers
    }
}

/// Linggen Cloud's voice uses — live voice and read-aloud — as lines on its
/// card, ready when a subscription is known (no network). The engine's own
/// wires decide them (`live`, `account::cloud_voice`), not a list.
fn cloud_uses() -> Vec<ProviderModel> {
    let live = crate::live::PROVIDERS.iter().find(|p| p.via_cloud())
        .map(|p| (Kind::LiveVoice, p.model, crate::account::cloud_voice::live_usable()));
    let speak = Some((Kind::Speak, crate::account::cloud_voice::SPEECH_MODEL, crate::account::cloud_voice::speech_usable()));
    live.into_iter().chain(speak).map(|(kind, model, ready)| {
        let id = format!("{}:cloud:{model}", kind.as_str());
        // Signed in without a subscription: not ready, and no reason a sign-in would fix.
        let why = (!ready && crate::account::resolve_token().is_none()).then_some(NotReady::NotSignedIn);
        ProviderModel { id, model: model.into(), kind: kind.as_str(), ready, why, builtin: true, name: model.into(), first: false }
    }).collect()
}

/// Linggen Cloud's chat models, each with the row that runs it when ticked.
fn cloud_lines(config: &Config) -> Vec<CloudLine> {
    super::cloud_models::current().models.into_iter().map(|m| CloudLine {
        row: super::cloud_tick::ticked(config, &m.id).map(|r| r.id.clone()),
        model: m.id,
        label: m.label,
    }).collect()
}

/// The preset a live voice's key is filed at: the one at its host.
fn live_preset(voice: &crate::live::Provider) -> Option<&'static presets::Preset> {
    presets::PRESETS.iter().find(|p| p.way == Way::Key && crate::credentials::url_host(p.url).is_some_and(|h| h.eq_ignore_ascii_case(voice.key_host())))
}

/// What the page offers to add, in the table's order (`presets::PRESETS`).
pub fn presets() -> Vec<serde_json::Value> {
    presets::PRESETS.iter().map(|p| serde_json::json!({
        "name": p.name, "label": p.label, "url": p.url, "way": p.way.as_str(), "style": p.style.as_str(),
    })).collect()
}

/// Where a live voice's card sits when it has no row or key yet: the preset at its host.
pub fn live_card_at(id: &str) -> Option<(String, String)> {
    let preset = live::offered_to_pages().and_then(live_preset)?;
    (provider_id(SignIn::Key, preset.name, preset.url) == id).then(|| (preset.name.to_string(), preset.url.to_string()))
}

/// Linggen Cloud's card has no row or key to find it by: its id is the account's.
pub fn linggen_cloud_card_at(id: &str) -> Option<(String, String)> {
    let (name, url) = (super::catalog::LINGGEN_CLOUD_PROVIDER, super::catalog::linggen_cloud_url());
    (provider_id(SignIn::LinggenAccount, name, &url) == id).then(|| (name.to_string(), url))
}

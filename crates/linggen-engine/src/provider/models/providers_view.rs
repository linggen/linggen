//! Settings' provider groups: a projection of the existing rows and endpoint keys.

use super::{ModelManager, NotReady};
use crate::config::providers::{label_for, Way};
use crate::config::{Config, SignIn};
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
    connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    fix: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<KeyState>,
    pub models: Vec<ProviderModel>,
    used_by: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    disk_gb: Option<u64>,
}

#[derive(Serialize)]
struct KeyState {
    set: bool,
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
}

/// Identify the filed pair using the same normalization as credential matching.
pub fn provider_id(sign_in: SignIn, name: &str, url: &str) -> String {
    let class = match sign_in {
        SignIn::Key => "key",
        SignIn::ChatGpt => "chatgpt",
        SignIn::Claude => "claude",
        SignIn::LinggenAccount => "account",
        SignIn::Unknown => "unknown",
    };
    let parts = (class, name.trim().to_ascii_lowercase(), credentials::endpoint_url(url).to_ascii_lowercase());
    format!("{:x}", Sha256::digest(serde_json::to_vec(&parts).expect("strings serialize")))
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
        let (label, way) = match sign_in {
            SignIn::LinggenAccount => ("Linggen Cloud".into(), Way::Account),
            SignIn::ChatGpt => ("ChatGPT 订阅".into(), Way::Subscription),
            SignIn::Claude => ("Claude 订阅".into(), Way::Subscription),
            SignIn::Key | SignIn::Unknown => {
                let way = if provider.eq_ignore_ascii_case("ollama") { Way::Local } else { Way::Key };
                (label_for(provider, url), way)
            }
        };
        Self {
            id: provider_id(sign_in, provider, url), label, way: way.as_str(),
            provider: provider.into(), url: shown_url(url), connected: false, fix: None,
            key: (way == Way::Key).then_some(KeyState { set: false }),
            models: Vec::new(), used_by: Vec::new(), disk_gb: None,
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
        groups.insert(cloud.id.clone(), cloud);
        let rows = self.list_models();
        for row in rows.iter().copied().filter(|row| row.is_builtin)
            .chain(config.chat_models()).chain(self.live_rows())
        {
            let sign_in = row.sign_in();
            let id = provider_id(sign_in, row.provider_name(), &row.url);
            let group = groups.entry(id).or_insert_with(|| ProviderView::new(sign_in, row.provider_name(), &row.url));
            if group.models.iter().any(|m| m.id == row.id) {
                continue;
            }
            group.mark_key(credentials::resolve_api_key(row, creds).is_some());
            let kind = kind_of(row);
            let why = match kind {
                Kind::Chat => {
                    if rows.iter().any(|m| m.id == row.id && m.is_builtin == row.is_builtin) {
                        self.readiness(&row.id).and_then(|r| r.reason)
                    } else {
                        Some(NotReady::ProviderFailing)
                    }
                }
                Kind::LiveVoice => {
                    let ready = credentials::url_host(&row.url)
                        .and_then(|host| live::at_model(&host, &row.model))
                        .is_some_and(|p| live::key_ready(p, &config.models, creds));
                    (!ready).then_some(NotReady::NoKey)
                }
                Kind::Speak | Kind::Hear | Kind::Draw => unreachable!("not configured rows"),
            };
            group.add(ProviderModel { id: row.id.clone(), model: row.model.clone(), kind: kind.as_str(), ready: why.is_none(), why, builtin: row.is_builtin });
        }
        for endpoint in &creds.endpoints {
            // A credential has no sign-in field; only the key group can use it.
            let id = provider_id(SignIn::Key, &endpoint.provider, &endpoint.url);
            let group = groups.entry(id).or_insert_with(|| ProviderView::new(SignIn::Key, &endpoint.provider, &endpoint.url));
            let set = creds.endpoint_key(&endpoint.provider, &endpoint.url).is_some();
            group.mark_key(set);
            group.connected |= set && group.models.is_empty();
        }
        let mut providers: Vec<_> = groups.into_values().collect();
        for group in &mut providers {
            group.models.sort_by(|a, b| a.id.cmp(&b.id));
            group.used_by.sort_unstable();
            group.fix = match group.way {
                "key" if group.key.as_ref().is_some_and(|k| !k.set) && !group.connected => Some("add_key"),
                "account" | "subscription" if group.fix.is_some() || group.models.iter().any(|m| matches!(m.why, Some(NotReady::NotSignedIn | NotReady::SignInExpired | NotReady::ProviderSignIn))) => Some("sign_in"),
                _ => None,
            };
        }
        providers.extend(local_lanes());
        providers.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.id.cmp(&b.id)));
        providers
    }
}

fn local_lanes() -> impl Iterator<Item = ProviderView> {
    use runtime::{HUB_MODELS, PICTURES_LANE, PICTURE_MODEL, STT_MODEL, TTS_MODEL, VOICE_LANE};
    [
        (Kind::Speak, TTS_MODEL, VOICE_LANE, runtime::voice_ready()),
        (Kind::Hear, STT_MODEL, VOICE_LANE, runtime::hearing_ready()),
        (Kind::Draw, PICTURE_MODEL, PICTURES_LANE, runtime::pictures_ready()),
    ].into_iter().map(|(kind, model, lane, on_disk)| {
        let ready = on_disk && runtime::lane_verdict(&lane).is_ok();
        let hub = HUB_MODELS.iter().find(|m| m.repo == model).expect("local model is declared");
        let mut group = ProviderView::new(SignIn::Key, "ollama", "");
        group.id = provider_id(SignIn::Key, "ollama", kind.as_str());
        group.label = "This Mac".into();
        group.fix = (!ready).then(|| if runtime::env_bin(hub.venv, "python3").exists() { "download" } else { "install" });
        group.disk_gb = Some(hub.need_gb);
        group.add(ProviderModel { id: kind.as_str().into(), model: model.into(), kind: kind.as_str(), ready, why: None, builtin: true });
        group
    })
}

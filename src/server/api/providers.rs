//! Owner-only provider settings. The engine projects the stored rows and keys.

use std::sync::Arc;

use axum::{body::Bytes, extract::{Path, State}, http::{HeaderMap, Method, StatusCode}, response::{IntoResponse, Response}, Json};
use serde::Deserialize;
use serde_json::json;

use crate::config::{providers::{known_names, Way, PRESETS}, Config, ModelConfig, SignIn};
use crate::provider::models::providers_view::{provider_id, ProviderView};
use crate::credentials::{self, Credentials};
use crate::server::{person, ServerState};

pub(crate) async fn list(State(state): State<Arc<ServerState>>, headers: HeaderMap) -> Response {
    if let Some(refused) = owner_refusal(&headers) {
        return refused;
    }
    let (config, creds, response) = {
        let _write = state.settings_write.lock().await;
        let config = state.manager.get_config_snapshot().await;
        let creds = Credentials::load_with(&credentials::credentials_file(), &config.models);
        let response = answer(&state, &config, &creds).await;
        (config, creds, response)
    };
    tokio::spawn(async move { state.manager.families.gather(&config, &creds, None).await });
    response
}

pub(super) fn owner_refusal(headers: &HeaderMap) -> Option<Response> {
    (!person::of_headers(headers).is_owner()).then(|| StatusCode::FORBIDDEN.into_response())
}

pub(super) async fn answer(state: &ServerState, config: &Config, creds: &Credentials) -> Response {
    let mut providers = state.manager.models.read().await.providers_view(config, creds);
    let drafts = state.provider_drafts.lock().await;
    for (id, (name, url)) in drafts.iter() {
        if !providers.iter().any(|p| &p.id == id) {
            providers.push(ProviderView::draft(name, url));
        }
    }
    for provider in providers.iter_mut().filter(|p| p.way == Way::Key.as_str()) {
        provider.families = state.manager.families.view(config, creds, (&provider.provider, &provider.url));
    }
    let presets: Vec<_> = PRESETS.iter().map(|p| json!({
        "name": p.name, "label": p.label, "url": p.url, "way": p.way.as_str(),
        "default_model": p.default_model, "placeholder": p.placeholder, "live": p.live_models(),
    })).collect();
    Json(json!({"presets": presets, "providers": providers})).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewProvider {
    preset: Option<String>,
    name: Option<String>,
    url: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyEdit { api_key: serde_json::Value }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddressEdit { url: String }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewModel {
    /// Left out by the page: the engine makes one (`Config::fresh_model_id`).
    id: Option<String>,
    model: String,
}

pub(super) type Failure = (StatusCode, &'static str);
pub(super) const BAD: Failure = (StatusCode::BAD_REQUEST, "Invalid provider settings.");
pub(super) const MISSING: Failure = (StatusCode::NOT_FOUND, "Provider or model not found.");
pub(super) const SAVE: Failure = (StatusCode::INTERNAL_SERVER_ERROR, "Could not save provider settings.");
const CONFLICT: Failure = (StatusCode::CONFLICT, "That provider address or model ID is already in use.");

pub(super) fn parse<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, Failure> {
    serde_json::from_slice(body).map_err(|_| BAD)
}

fn address(input: &str) -> Result<String, Failure> {
    let url = url::Url::parse(input.trim()).map_err(|_| BAD)?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none()
        || !url.username().is_empty() || url.password().is_some()
        || url.query().is_some() || url.fragment().is_some()
    {
        return Err(BAD);
    }
    Ok(input.trim().into())
}

fn new_endpoint(input: NewProvider) -> Result<(String, String), Failure> {
    match (input.preset, input.name, input.url) {
        (Some(selector), None, None) => {
            let preset = PRESETS.iter().find(|p| p.label == selector || p.url == selector)
                .or_else(|| PRESETS.iter().find(|p| p.name == selector)).ok_or(BAD)?;
            if !matches!(preset.way, Way::Key | Way::Local) { return Err(BAD); }
            Ok((preset.name.into(), preset.url.into()))
        }
        (None, Some(name), Some(url)) if known_names().contains(&name.as_str()) && name != "chatgpt" => {
            Ok((name, address(&url)?))
        }
        _ => Err(BAD),
    }
}

pub(crate) async fn create(State(state): State<Arc<ServerState>>, headers: HeaderMap, body: Bytes) -> Response {
    mutate(state, headers, Edit::Create(body)).await
}

pub(crate) async fn edit(State(state): State<Arc<ServerState>>, headers: HeaderMap, method: Method, Path(id): Path<String>, body: Bytes) -> Response {
    let edit = match method {
        Method::PUT => return put_key(state, headers, id, body).await,
        Method::PATCH => Edit::Address(id, body),
        Method::POST => Edit::AddModel(id, body),
        _ => return StatusCode::METHOD_NOT_ALLOWED.into_response(),
    };
    mutate(state, headers, edit).await
}

/// File an endpoint's key; then, outside the write lock, ask its provider for
/// the models it lists; then, once per endpoint, tick the preset's starter
/// families (`families::Families::start_endpoint`). A key the provider does
/// not accept ticks nothing and records nothing.
async fn put_key(state: Arc<ServerState>, headers: HeaderMap, id: String, body: Bytes) -> Response {
    if let Some(refused) = owner_refusal(&headers) { return refused; }
    let filed = {
        let _write = state.settings_write.lock().await;
        apply(&state, Edit::Key(id.clone(), body)).await
    };
    let (config, creds) = match filed {
        Ok(filed) => filed,
        Err(failure) => return failure.into_response(),
    };
    let Ok((name, url)) = endpoint(&state, &config, &creds, &id).await else {
        return answer(&state, &config, &creds).await;
    };
    state.manager.families.gather(&config, &creds, Some((&name, &url))).await;
    let _write = state.settings_write.lock().await;
    let mut config = state.manager.get_config_snapshot().await;
    let creds = Credentials::load_with(&credentials::credentials_file(), &config.models);
    if state.manager.families.start_endpoint(&mut config, &creds, (&name, &url)) {
        if let Err(failure) = save_config(&state, &config).await { return failure.into_response(); }
    }
    answer(&state, &config, &creds).await
}

pub(crate) async fn remove_model(State(state): State<Arc<ServerState>>, headers: HeaderMap, Path((id, model)): Path<(String, String)>) -> Response {
    mutate(state, headers, Edit::RemoveModel(id, model)).await
}

enum Edit {
    Create(Bytes),
    Key(String, Bytes),
    Address(String, Bytes),
    AddModel(String, Bytes),
    RemoveModel(String, String),
}

async fn mutate(state: Arc<ServerState>, headers: HeaderMap, edit: Edit) -> Response {
    if let Some(refused) = owner_refusal(&headers) { return refused; }
    let _write = state.settings_write.lock().await;
    match apply(&state, edit).await {
        Ok((config, creds)) => answer(&state, &config, &creds).await,
        Err(failure) => failure.into_response(),
    }
}

fn matches_group(row: &ModelConfig, id: &str) -> bool {
    !row.is_builtin && row.sign_in() == SignIn::Key && provider_id(row.sign_in(), &row.provider, &row.url) == id
}

pub(super) async fn endpoint(state: &ServerState, config: &Config, creds: &Credentials, id: &str) -> Result<(String, String), Failure> {
    let filed = config.models.iter().find(|row| matches_group(row, id))
        .map(|row| (row.provider.clone(), row.url.clone()))
        .or_else(|| creds.endpoints.iter().find(|e| provider_id(SignIn::Key, &e.provider, &e.url) == id)
            .map(|e| (e.provider.clone(), e.url.clone())));
    let pair = match filed {
        Some(pair) => pair,
        None => {
            let draft = state.provider_drafts.lock().await.get(id).cloned();
            draft.or_else(|| PRESETS.iter().find(|p| !p.live_models().is_empty()
                && provider_id(SignIn::Key, p.name, p.url) == id)
                .map(|p| (p.name.into(), p.url.into()))).ok_or(MISSING)?
        },
    };
    if !known_names().contains(&pair.0.as_str()) || pair.0 == "chatgpt" { return Err(BAD); }
    Ok(pair)
}

async fn apply(state: &Arc<ServerState>, edit: Edit) -> Result<(Config, Credentials), Failure> {
    let mut config = state.manager.get_config_snapshot().await;
    let path = credentials::credentials_file();
    if let Edit::Create(body) = edit {
        let creds = Credentials::load_with(&path, &config.models);
        let (name, url) = new_endpoint(parse(&body)?)?;
        let mut drafts = state.provider_drafts.lock().await;
        if drafts.len() >= 64 { return Err(CONFLICT); }
        drafts.insert(provider_id(SignIn::Key, &name, &url), (name, url));
        return Ok((config, creds));
    }
    let mut creds = Credentials::load_with(&path, &config.models);
    let id = match &edit {
        Edit::Key(id, _) | Edit::Address(id, _) | Edit::AddModel(id, _) | Edit::RemoveModel(id, _) => id,
        Edit::Create(_) => unreachable!(),
    };
    let (name, url) = endpoint(state, &config, &creds, id).await?;
    match edit {
        Edit::Key(id, body) => {
            address(&url)?;
            if name == "ollama" { return Err(BAD); }
            let key = match parse::<KeyEdit>(&body)?.api_key {
                serde_json::Value::Null => None,
                serde_json::Value::String(key) if !key.trim().is_empty() && key != "***" => Some(key),
                _ => return Err(BAD),
            };
            creds.set_endpoint_key(&name, &url, key);
            creds.save(&path).map_err(|_| SAVE)?;
            super::config::reload_models(state).await;
            state.provider_drafts.lock().await.remove(&id);
        }
        Edit::Address(id, body) => {
            let new = address(&parse::<AddressEdit>(&body)?.url)?;
            let next_id = provider_id(SignIn::Key, &name, &new);
            if next_id != id {
                if endpoint(state, &config, &creds, &next_id).await.is_ok() { return Err(CONFLICT); }
                let mut has_rows = false;
                for row in config.models.iter_mut().filter(|row| matches_group(row, &id)) {
                    row.url = new.clone();
                    has_rows = true;
                }
                validate_config(&config)?;
                let key = creds.endpoint_key(&name, &url).map(str::to_owned);
                if key.is_some() {
                    // Copy first. A failed config save leaves the old key usable.
                    creds.set_endpoint_key(&name, &new, key);
                    creds.save(&path).map_err(|_| SAVE)?;
                }
                if has_rows { save_config(state, &config).await?; }
                if creds.endpoint_key(&name, &url).is_some() {
                    // Only after the new URL has been filed do we drop the old key.
                    creds.set_endpoint_key(&name, &url, None);
                    creds.save(&path).map_err(|_| SAVE)?;
                }
                let mut drafts = state.provider_drafts.lock().await;
                if drafts.remove(&id).is_some() && !has_rows && creds.endpoint_key(&name, &new).is_none() {
                    drafts.insert(next_id, (name, new));
                }
            }
        }
        Edit::AddModel(id, body) => {
            let model = parse::<NewModel>(&body)?;
            address(&url)?;
            let row_id = model.id.unwrap_or_else(|| config.fresh_model_id("own"));
            let row = serde_json::from_value(json!({"id": row_id, "model": model.model, "provider": name, "url": url})).map_err(|_| BAD)?;
            config.models.push(row);
            save_config(state, &config).await?;
            state.provider_drafts.lock().await.remove(&id);
        }
        Edit::RemoveModel(id, model) => {
            let index = config.models.iter().position(|row| row.id == model && matches_group(row, &id)).ok_or(MISSING)?;
            // A family's row is removed by unticking, which names a replacement
            // when the row is the first default chat model.
            if config.models[index].family_binding.is_some() && config.routing.default_models.first() == Some(&model) {
                return Err((StatusCode::CONFLICT, "first_default"));
            }
            let row = config.models.remove(index);
            validate_config(&config)?;
            if !config.models.iter().any(|row| matches_group(row, &id)) && creds.endpoint_key(&name, &url).is_none() {
                let key = row.api_key.as_deref().filter(|k| !k.is_empty() && *k != "***")
                    .or_else(|| creds.model_override(&model)).map(str::to_owned);
                if key.is_some() {
                    creds.set_endpoint_key(&name, &url, key);
                    creds.save(&path).map_err(|_| SAVE)?;
                }
            }
            save_config(state, &config).await?;
            if creds.prune_overrides(&config.models) > 0 {
                creds.save(&path).map_err(|_| SAVE)?;
            }
        }
        Edit::Create(_) => unreachable!(),
    }
    Ok((config, creds))
}

fn validate_config(config: &Config) -> Result<(), Failure> {
    config.validate().map_err(|error| {
        if error.is::<crate::config::DuplicateModelId>() {
            (StatusCode::CONFLICT, "duplicate_model_id")
        } else {
            BAD
        }
    })
}

pub(super) async fn save_config(state: &Arc<ServerState>, config: &Config) -> Result<(), Failure> {
    validate_config(config)?;
    state.manager.apply_config(config.clone()).await.map_err(|_| SAVE)?;
    crate::server::rtc::proxy_room::reapply_proxy_models(state).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_settings_need_the_owner_not_just_an_admin_level() {
        assert!(owner_refusal(&HeaderMap::new()).is_none());
        let mut headers = HeaderMap::new();
        headers.insert(crate::server::api::pair::LEVEL_HEADER, "admin".parse().unwrap());
        assert_eq!(owner_refusal(&headers).unwrap().status(), StatusCode::FORBIDDEN);
    }
}

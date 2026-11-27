//! Owner-only provider settings. The engine projects the stored rows and keys.

use std::sync::Arc;

use axum::{body::Bytes, extract::{Path, State}, http::{HeaderMap, Method, StatusCode}, response::{IntoResponse, Response}, Json};
use serde::Deserialize;
use serde_json::json;

use crate::config::{providers::{known_names, Way}, Config, SignIn};
use crate::provider::models::presets::{self as preset_table, ApiStyle};
use crate::provider::models::providers_view::{editable_row, is_listed_address, chatgpt_card_id, linggen_cloud_card_at, live_card_at, presets, filed_endpoint, group_keys, keep_model_key, provider_id, ProviderView};
use crate::provider::models::{cloud_tick, suggest};
use crate::credentials::{self, Credentials};
use crate::server::{person, ServerState};

pub(crate) async fn list(State(state): State<Arc<ServerState>>, headers: HeaderMap) -> Response {
    if let Some(refused) = owner_refusal(&headers) {
        return refused;
    }
    let _write = state.settings_write.lock().await;
    let config = state.manager.get_config_snapshot().await;
    let creds = Credentials::load_with(&credentials::credentials_file(), &config.models);
    answer(&state, &config, &creds).await
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
    Json(json!({"presets": presets(), "providers": providers})).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewProvider {
    preset: Option<String>,
    /// 其他…: the person's own name for it (a label), its address and API style.
    name: Option<String>,
    url: Option<String>,
    style: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum KeyEdit {
    Set { api_key: serde_json::Value },
    Clean { action: KeyAction },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum KeyAction { KeepModelKey }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddressEdit {
    url: String,
    /// The person's answer to 「地址改了，密钥还用吗？」; true when not asked.
    #[serde(default = "yes")]
    keep_key: bool,
}

fn yes() -> bool { true }


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
    match (input.preset, input.name, input.url, input.style) {
        (Some(selector), None, None, None) => {
            let preset = preset_table::named(&selector).ok_or(BAD)?;
            if !matches!(preset.way, Way::Key | Way::Local) { return Err(BAD); }
            Ok((preset.name.to_string(), preset.url.to_string()))
        }
        // A provider the person types: filed under the name its API style speaks.
        (None, Some(name), Some(url), Some(style)) if !name.trim().is_empty() => {
            let style = ApiStyle::parse(&style).ok_or(BAD)?;
            Ok((preset_table::name_for_style(style).to_string(), address(&url)?))
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

/// File, replace or clean an endpoint's key. Nothing is ticked for it: the
/// person adds the models they want.
async fn put_key(state: Arc<ServerState>, headers: HeaderMap, id: String, body: Bytes) -> Response {
    match parse::<KeyEdit>(&body) {
        Ok(edit) => mutate(state, headers, Edit::Key(id, edit)).await,
        Err(failure) => failure.into_response(),
    }
}

/// `GET /api/providers/{id}/suggestions`: the ids this provider lists for its
/// key, to suggest under the model input. Asked outside the write lock; a
/// list that cannot be read answers `{"models": null}` and the person types.
pub(crate) async fn suggestions(State(state): State<Arc<ServerState>>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if let Some(refused) = owner_refusal(&headers) { return refused; }
    let config = state.manager.get_config_snapshot().await;
    let creds = Credentials::load_with(&credentials::credentials_file(), &config.models);
    let Ok((name, url)) = endpoint(&state, &config, &creds, &id).await else { return MISSING.into_response() };
    let probe: crate::config::ModelConfig = match serde_json::from_value(json!({"id": "-", "provider": name, "url": url, "model": "-"})) {
        Ok(row) => row,
        Err(_) => return BAD.into_response(),
    };
    let wire = match probe.provider_kind() {
        crate::config::ProviderKind::Anthropic => suggest::Wire::Anthropic,
        crate::config::ProviderKind::Ollama => suggest::Wire::Ollama,
        _ => suggest::Wire::OpenAi,
    };
    let key = credentials::resolve_api_key(&probe, &creds);
    let base = if url.trim().is_empty() && wire == suggest::Wire::Anthropic { crate::provider::claude_auth::ANTHROPIC_API_BASE.to_string() } else { url };
    Json(json!({"models": suggest::list(wire, &base, key.as_deref()).await})).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CloudTick { model: String, ticked: bool, #[serde(default)] replacement: Option<String> }

/// `PUT /api/providers/cloud`: tick or untick one of Linggen Cloud's models.
/// Unticking the first default needs the model that takes its place.
pub(crate) async fn tick_cloud(State(state): State<Arc<ServerState>>, headers: HeaderMap, body: Bytes) -> Response {
    match parse::<CloudTick>(&body) {
        Ok(tick) => mutate(state, headers, Edit::Cloud(tick)).await,
        Err(failure) => failure.into_response(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatgptTick { family: String, ticked: bool, #[serde(default)] replacement: Option<String> }

/// `PUT /api/providers/chatgpt`: tick or untick one of the ChatGPT sign-in's models
/// (by family). Unticking the first default needs the model that takes its place.
pub(crate) async fn tick_chatgpt(State(state): State<Arc<ServerState>>, headers: HeaderMap, body: Bytes) -> Response {
    match parse::<ChatgptTick>(&body) {
        Ok(tick) => mutate(state, headers, Edit::Chatgpt(tick)).await,
        Err(failure) => failure.into_response(),
    }
}

/// `DELETE /api/providers/{id}`: the provider, its key and every model row
/// filed under it. The page confirms first, naming any default it holds.
pub(crate) async fn remove(State(state): State<Arc<ServerState>>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    mutate(state, headers, Edit::Remove(id)).await
}

pub(crate) async fn remove_model(State(state): State<Arc<ServerState>>, headers: HeaderMap, Path((id, model)): Path<(String, String)>) -> Response {
    mutate(state, headers, Edit::RemoveModel(id, model)).await
}

enum Edit {
    Create(Bytes),
    Cloud(CloudTick),
    Chatgpt(ChatgptTick),
    Remove(String),
    Key(String, KeyEdit),
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

pub(super) async fn endpoint(state: &ServerState, config: &Config, creds: &Credentials, id: &str) -> Result<(String, String), Failure> {
    let filed = filed_endpoint(config, creds, id).map(|(name, url)| (name.to_owned(), url.to_owned()))
        .or_else(|| linggen_cloud_card_at(id));
    let pair = match filed {
        Some(pair) => pair,
        None => {
            let draft = state.provider_drafts.lock().await.get(id).cloned();
            // A live voice's card has no row or key yet: the list names where it is shown.
            let live = live_card_at(id);
            draft.or(live).ok_or(MISSING)?
        },
    };
    if !known_names().contains(&pair.0.as_str()) || pair.0 == "chatgpt" { return Err(BAD); }
    Ok(pair)
}

async fn apply(state: &Arc<ServerState>, edit: Edit) -> Result<(Config, Credentials), Failure> {
    let mut config = state.manager.get_config_snapshot().await;
    let path = credentials::credentials_file();
    if let Edit::Cloud(CloudTick { model, ticked, replacement }) = edit {
        let creds = Credentials::load_with(&path, &config.models);
        if !crate::provider::models::cloud_models::current().models.iter().any(|m| m.id == model) { return Err(MISSING); }
        let changed = if ticked {
            cloud_tick::tick(&mut config, &model)
        } else {
            let Some(id) = cloud_tick::ticked(&config, &model).map(|r| r.id.clone()) else { return Ok((config, creds)) };
            unfile_rows(&mut config, &[id], replacement)?;
            true
        };
        if changed { save_config(state, &config).await?; }
        return Ok((config, creds));
    }
    if let Edit::Chatgpt(tick) = edit {
        let creds = Credentials::load_with(&path, &config.models);
        tick_chatgpt_family(state, &mut config, tick).await?;
        return Ok((config, creds));
    }
    if let Edit::Remove(id) = &edit {
        if *id == chatgpt_card_id() {
            let creds = Credentials::load_with(&path, &config.models);
            sign_out_chatgpt(state, &mut config).await?;
            return Ok((config, creds));
        }
    }
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
        Edit::Key(id, _) | Edit::Address(id, _) | Edit::AddModel(id, _) | Edit::RemoveModel(id, _) | Edit::Remove(id) => id,
        Edit::Create(_) | Edit::Cloud(_) | Edit::Chatgpt(_) => unreachable!(),
    };
    let (name, url) = endpoint(state, &config, &creds, id).await?;
    match edit {
        Edit::Key(id, body) => {
            address(&url)?;
            if name == "ollama" { return Err(BAD); }
            match body {
                KeyEdit::Clean { action: KeyAction::KeepModelKey } => {
                    // The same guard that offered the action, rechecked under the write lock.
                    creds = keep_model_key(&config, &creds, &id).ok_or(CONFLICT)?;
                }
                KeyEdit::Set { api_key } => {
                    let key = match api_key {
                        serde_json::Value::Null => None,
                        serde_json::Value::String(key) if !key.trim().is_empty() && key != "***" => Some(key),
                        _ => return Err(BAD),
                    };
                    // One card holds one key: a replace or remove reaches every name filed there,
                    // so a row filed under any of those names keeps finding the card's key.
                    let filed: Vec<_> = group_keys(&creds, &id).map(|e| (e.provider.clone(), e.url.clone())).collect();
                    for (other, at) in &filed { creds.set_endpoint_key(other, at, key.clone()); }
                    if !filed.iter().any(|(other, at)| credentials::same_endpoint(other, at, &name, &url)) {
                        creds.set_endpoint_key(&name, &url, key);
                    }
                }
            }
            creds.save(&path).map_err(|_| SAVE)?;
            super::config::reload_models(state).await;
            state.provider_drafts.lock().await.remove(&id);
        }
        Edit::Address(id, body) => {
            let AddressEdit { url: new, keep_key } = parse::<AddressEdit>(&body)?;
            // The built-in list owns its addresses; only an added provider's moves.
            if is_listed_address(&url) { return Err(BAD); }
            let new = address(&new)?;
            let next_id = provider_id(SignIn::Key, &name, &new);
            if next_id != id {
                if endpoint(state, &config, &creds, &next_id).await.is_ok() { return Err(CONFLICT); }
                let mut has_rows = false;
                for row in config.models.iter_mut().filter(|row| editable_row(row, &id)) {
                    row.url = new.clone();
                    has_rows = true;
                }
                validate_config(&config)?;
                let keys: Vec<_> = group_keys(&creds, &id).cloned().collect();
                if keep_key && !keys.is_empty() {
                    // Copy first. A failed config save leaves the old keys usable.
                    for key in &keys {
                        creds.set_endpoint_key(&key.provider, &new, key.api_key.clone());
                    }
                    creds.save(&path).map_err(|_| SAVE)?;
                }
                if has_rows { save_config(state, &config).await?; }
                if !keys.is_empty() {
                    // Only after the new URL has been filed do we drop the old keys.
                    for key in &keys {
                        creds.set_endpoint_key(&key.provider, &key.url, None);
                    }
                    creds.save(&path).map_err(|_| SAVE)?;
                }
                let mut drafts = state.provider_drafts.lock().await;
                if drafts.remove(&id).is_some() && !has_rows && keys.is_empty() {
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
            let index = config.models.iter().position(|row| row.id == model && editable_row(row, &id)).ok_or(MISSING)?;
            let row = config.models.remove(index);
            validate_config(&config)?;
            let (name, url) = (&row.provider, &row.url);
            if !config.models.iter().any(|r| editable_row(r, &id)
                && credentials::same_endpoint(&r.provider, &r.url, name, url)) && creds.endpoint_key(name, url).is_none() {
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
        Edit::Remove(id) => {
            let rows: Vec<String> = config.models.iter().filter(|r| editable_row(r, &id)).map(|r| r.id.clone()).collect();
            // The page named the defaults it holds and the person confirmed:
            // the chain goes on without them.
            config.models.retain(|row| !rows.contains(&row.id));
            config.routing.default_models.retain(|m| !rows.contains(m));
            save_config(state, &config).await?;
            for (other, at) in group_keys(&creds, &id).map(|e| (e.provider.clone(), e.url.clone())).collect::<Vec<_>>() {
                creds.set_endpoint_key(&other, &at, None);
            }
            creds.prune_overrides(&config.models);
            creds.save(&path).map_err(|_| SAVE)?;
            super::config::reload_models(state).await;
            state.provider_drafts.lock().await.remove(&id);
        }
        Edit::Create(_) | Edit::Cloud(_) | Edit::Chatgpt(_) => unreachable!(),
    }
    Ok((config, creds))
}

/// The ids of the ChatGPT models a family stands for now.
fn chatgpt_ids_of(family: &str) -> Vec<String> {
    let catalog = crate::provider::models::chatgpt_catalog::current();
    catalog.family(family).map(|f| vec![catalog.effective(f).to_string()]).unwrap_or_default()
}

/// Tick or untick a ChatGPT family. Unticking the first default's family needs a `replacement`.
async fn tick_chatgpt_family(state: &Arc<ServerState>, config: &mut Config, tick: ChatgptTick) -> Result<(), Failure> {
    let ids = chatgpt_ids_of(&tick.family);
    if ids.is_empty() || !crate::provider::codex_auth::chatgpt_signed_in() { return Err(MISSING); }
    let was = config.chatgpt_unticked.contains(&tick.family);
    if tick.ticked {
        config.chatgpt_unticked.retain(|f| *f != tick.family);
    } else if !was {
        let first = config.routing.default_models.first().is_some_and(|d| ids.contains(d));
        if first {
            let replacement = tick.replacement.filter(|r| !r.is_empty() && !ids.contains(r)).ok_or((StatusCode::CONFLICT, "first_default"))?;
            config.make_default(&replacement);
        }
        config.routing.default_models.retain(|m| !ids.contains(m));
        config.chatgpt_unticked.push(tick.family);
    }
    // Saved only when the tick changed something.
    if tick.ticked == was { save_config(state, config).await?; }
    Ok(())
}

/// 移除这个服务商 for the ChatGPT sign-in: sign out, forget the unticks, and let the
/// defaults go on without its models. The page confirmed, naming any default it holds.
async fn sign_out_chatgpt(state: &Arc<ServerState>, config: &mut Config) -> Result<(), Failure> {
    let catalog = crate::provider::models::chatgpt_catalog::current();
    let mine = catalog.builtin_ids();
    config.routing.default_models.retain(|m| !mine.contains(m));
    config.chatgpt_unticked.clear();
    crate::provider::codex_auth::CodexAuthManager::new().logout().await.map_err(|_| SAVE)?;
    save_config(state, config).await
}

/// Untick rows. The first default chat model may go only with a
/// `replacement` to take its place; a pin naming a row that went simply
/// finds it unavailable.
fn unfile_rows(config: &mut Config, ids: &[String], replacement: Option<String>) -> Result<(), Failure> {
    if config.routing.default_models.first().is_some_and(|first| ids.contains(first)) {
        let replacement = replacement.filter(|r| !r.is_empty() && !ids.contains(r)).ok_or((StatusCode::CONFLICT, "first_default"))?;
        config.make_default(&replacement);
    }
    config.models.retain(|row| !ids.contains(&row.id));
    config.routing.default_models.retain(|m| !ids.contains(m));
    Ok(())
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

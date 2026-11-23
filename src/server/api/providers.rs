//! Owner-only provider settings. The engine projects the stored rows and keys.

use std::sync::Arc;

use axum::{extract::State, http::{HeaderMap, StatusCode}, response::{IntoResponse, Response}, Json};
use serde_json::json;

use crate::config::{providers::PRESETS, Config};
use crate::credentials::{self, Credentials};
use crate::server::{person, ServerState};

pub(crate) async fn list(State(state): State<Arc<ServerState>>, headers: HeaderMap) -> Response {
    if let Some(refused) = owner_refusal(&headers) {
        return refused;
    }
    let config = state.manager.get_config_snapshot().await;
    let creds = Credentials::load(&credentials::credentials_file());
    answer(&state, &config, &creds).await
}

fn owner_refusal(headers: &HeaderMap) -> Option<Response> {
    (!person::of_headers(headers).is_owner()).then(|| StatusCode::FORBIDDEN.into_response())
}

async fn answer(state: &ServerState, config: &Config, creds: &Credentials) -> Response {
    let providers = state.manager.models.read().await.providers_view(config, creds);
    let presets: Vec<_> = PRESETS.iter().map(|p| json!({
        "name": p.name, "label": p.label, "url": p.url, "way": p.way.as_str(),
        "default_model": p.default_model, "placeholder": p.placeholder, "live": p.live_models(),
    })).collect();
    Json(json!({"presets": presets, "providers": providers})).into_response()
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

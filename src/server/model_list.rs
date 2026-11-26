//! The model list for a person or a room, before either wire projection.

use crate::config::ModelConfig;
use crate::provider::kinds::Kind;
use crate::server::{people::People, person::Person, ServerState};

/// One model of the list, with what the pickers need beside its settings.
pub(super) struct Listed {
    /// Its settings, never its key.
    pub config: ModelConfig,
    /// Whether its credentials are present now.
    pub auth_ok: bool,
    /// The effort values it offers (`ModelManager::effort_choices`): the one
    /// answer of both wire projections; the chat's effort chip lists these.
    pub effort: Vec<&'static str>,
    pub remembered_effort: Option<String>,
    pub name: String,
}

pub(super) async fn for_person(
    state: &ServerState,
    person: &Person,
    kind: Kind,
    people: &People,
    room_shared: Option<&[String]>,
) -> Vec<Listed> {
    let saved = state.manager.get_config_snapshot().await.uses.model_effort;
    let manager = state.manager.models.read().await;
    manager.list_of_kind(kind).into_iter()
        .filter(|m| match room_shared {
            Some(shared) => shared.contains(&m.id),
            None => people.may_run_model(person, &m.id),
        })
        .map(|m| {
            let auth_ok = match kind {
                Kind::Chat => manager.model_auth_ok(&m.id),
                // Live auth is reported by /api/live, not by a chat client.
                Kind::LiveVoice | Kind::Speak | Kind::Hear | Kind::Draw => false,
            };
            let mut config = m.clone();
            config.api_key = None;
            Listed { config, auth_ok, effort: manager.effort_choices(&m.id), remembered_effort: saved.get(&m.id).cloned(), name: manager.display_name(m) }
        })
        .collect()
}

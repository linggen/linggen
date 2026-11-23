//! The model list for a person or a room, before either wire projection.

use crate::config::ModelConfig;
use crate::provider::kinds::Kind;
use crate::server::{people::People, person::Person, ServerState};

pub(super) async fn for_person(
    state: &ServerState,
    person: &Person,
    kind: Kind,
    people: &People,
    room_shared: Option<&[String]>,
) -> Vec<(ModelConfig, bool)> {
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
            let mut shown = m.clone();
            shown.api_key = None;
            (shown, auth_ok)
        })
        .collect()
}

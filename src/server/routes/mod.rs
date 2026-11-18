//! The HTTP surface, one file per area. `router` puts them together; the
//! LAN gate is layered on by `prepare_server`.

mod apps;
mod chat;
pub(crate) mod classes;
mod devices;
mod rooms;
mod settings;
mod workspace;

use super::ServerState;
use axum::Router;
use std::sync::Arc;

type Routes = Router<Arc<ServerState>>;

pub(super) fn router(state: Arc<ServerState>) -> Router {
    Router::new()
        .merge(settings::routes())
        .merge(rooms::routes(super::rooms_switch::on()))
        .merge(chat::routes())
        .merge(workspace::routes())
        .merge(devices::routes())
        .merge(apps::routes())
        .merge(crate::memory_host::routes())
        .fallback(super::app_pages::static_handler)
        .with_state(state)
}

//! Files and the workspace, the shell, and media ingest.

use super::Routes;
use crate::server::api;
use crate::server::api::workspace::{
    get_agent_tree, get_workspace_state, list_files, read_file_api, run_bash_api, search_files,
};
use axum::routing::{get, post};

pub(super) fn routes() -> Routes {
    Routes::new()
        .route("/api/workspace/tree", get(get_agent_tree))
        .route("/api/files", get(list_files))
        .route("/api/files/search", get(search_files))
        .route("/api/file", get(read_file_api))
        .route("/api/workspace/state", get(get_workspace_state))
        .route("/api/bash", post(run_bash_api))
        .route("/api/media/manifest", post(api::media::manifest_handler))
        .route("/api/media/verify", post(api::media::verify_handler))
        .route("/api/media/reconcile", post(api::media::reconcile_handler))
        .route("/api/media/backup", post(api::media::backup_handler))
        .route(
            "/api/media/request-delete",
            post(api::media::request_delete_handler),
        )
        .route(
            "/api/media/pending-deletes",
            get(api::media::pending_deletes_handler),
        )
}

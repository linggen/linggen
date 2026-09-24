//! Skill apps: their pages, declared tools, cloud saves, capabilities, and
//! the shared page helpers.

use super::Routes;
use crate::server::app_pages::{capability_dispatch, serve_app_file};
use crate::server::{api, shared_pages};
use axum::routing::{get, post};

pub(super) fn routes() -> Routes {
    Routes::new()
        .route(
            "/api/skills/{skill}/tools/{tool}",
            post(api::skill_tools::run_tool),
        )
        .route("/api/skill-cloud/{skill}", get(api::skill_cloud::get_cloud))
        .route(
            "/api/skill-cloud/{skill}/sync",
            post(api::skill_cloud::post_sync),
        )
        .route(
            "/apps/{skill_name}/capability/{tool_name}",
            post(capability_dispatch),
        )
        .route("/shared/{file}", get(shared_pages::serve))
        .route("/apps/{skill_name}/{*file_path}", get(serve_app_file))
}

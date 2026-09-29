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
        // A sense a skill declares (`crate::senses`): its page reads it, and
        // sets the one thing the person gives it (the city).
        .route(
            "/api/senses/weather",
            get(crate::senses::weather::get_api).put(crate::senses::weather::put_api),
        )
        .route("/shared/{file}", get(shared_pages::serve))
        .route("/apps/{skill_name}/{*file_path}", get(serve_app_file))
}

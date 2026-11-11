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
        // The JS runtime skill actions run on (`crate::js_runtime`): GET reads
        // where it stands; POST also starts a fetch when it is missing.
        .route(
            "/api/runtime/js",
            get(|| async { axum::Json(crate::js_runtime::status()) }).post(|| async {
                crate::js_runtime::ensure_in_background();
                axum::Json(crate::js_runtime::status())
            }),
        )
        .route("/api/skill-cloud/{skill}", get(api::skill_cloud::get_cloud))
        .route(
            "/api/skill-cloud/{skill}/sync",
            post(api::skill_cloud::post_sync),
        )
        // A paired phone of the same account keeping its own play in step (skill_save.rs).
        .route(
            "/api/skill-save/{skill}",
            get(api::skill_save::get_save).put(api::skill_save::put_save),
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
        // One app's quest file, read (a paired phone's 开府 list).
        .route("/api/quests/{app}", get(api::quests::get_quests))
        .route("/shared/{file}", get(shared_pages::serve))
        .route("/apps/{skill_name}/{*file_path}", get(serve_app_file))
}

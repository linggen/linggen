//! Settings: agents, models, config, credentials, sign-ins, skills and the
//! marketplace, missions, rooms, account, storage and status.

use super::Routes;
use crate::server::api;
use crate::server::api::account::{
    get_account, get_account_callback, post_account_checkout, post_account_login,
    post_account_logout,
};
use crate::server::api::agents::{
    delete_agent_file_api, get_agent_file_api, list_agent_files_api, reload_agents,
    upsert_agent_file_api,
};
use crate::server::api::config::{
    codex_auth_logout, get_claude_auth_status, get_codex_auth_status, get_config_api,
    get_credentials_api, get_models_health, start_codex_auth_login, update_config_api,
    update_credentials_api,
};
use crate::server::api::marketplace::{
    builtin_skills_install, builtin_skills_list, community_search, marketplace_install,
    marketplace_move_to_global, marketplace_uninstall,
};
use crate::server::api::missions::{
    create_mission, delete_mission, get_mission_file, get_mission_session_state, list_mission_runs,
    list_missions, trigger_mission, update_mission, upsert_mission_file,
};
use crate::server::api::rooms::{
    connect_proxy_room_api, disconnect_proxy_room_api, get_room_config, proxy_rooms,
    proxy_status_api, token_usage_api, update_room_config,
};
use crate::server::api::skills::{
    delete_skill_file_api, get_skill_file_api, list_skill_files_api, list_skills, reload_skills,
    upsert_skill_file_api,
};
use crate::server::api::status::{get_status_api, list_models_api};
use crate::server::api::storage::{
    storage_delete_file, storage_read_file, storage_roots, storage_tree, storage_write_file,
};
use crate::server::api::utils::{get_ollama_status, get_user_name, health_handler};
use axum::routing::{any, delete, get, post, put};

pub(super) fn routes() -> Routes {
    Routes::new()
        // Agent & model file management (admin HTTP, not proxied for consumers)
        .route("/api/agent-files", get(list_agent_files_api))
        .route("/api/agent-file", get(get_agent_file_api))
        .route("/api/agent-file", post(upsert_agent_file_api))
        .route("/api/agent-file", delete(delete_agent_file_api))
        // Models & skills GETs — used by SharingTab and SkillsTab Settings pages directly.
        // Session list / agents / agent-runs come via page_state only (no GET route).
        .route("/api/models", get(list_models_api))
        .route("/api/mcp", get(api::config::list_mcp_api))
        // Memory for off-machine callers — the phone, over the WebRTC tunnel.
        // One route for all of ling-mem's verbs; see `api::memory`.
        .route("/api/memory/{verb}", post(api::memory::passthrough))
        .route("/api/skills", get(list_skills))
        .route("/api/models/health", get(get_models_health))
        .route("/api/config", get(get_config_api).post(update_config_api))
        .route(
            "/api/credentials",
            get(get_credentials_api).put(update_credentials_api),
        )
        .route("/api/auth/codex/status", get(get_codex_auth_status))
        .route("/api/auth/codex/login", post(start_codex_auth_login))
        .route("/api/auth/codex/logout", post(codex_auth_logout))
        .route("/api/auth/claude/status", get(get_claude_auth_status))
        .route("/api/skills/reload", post(reload_skills))
        .route("/api/agents/reload", post(reload_agents))
        .route("/api/community-skills/search", get(community_search))
        .route("/api/marketplace/install", post(marketplace_install))
        .route("/api/marketplace/uninstall", delete(marketplace_uninstall))
        .route(
            "/api/marketplace/move-to-global",
            post(marketplace_move_to_global),
        )
        .route("/api/builtin-skills", get(builtin_skills_list))
        .route("/api/builtin-skills/install", post(builtin_skills_install))
        .route("/api/skill-files", get(list_skill_files_api))
        .route("/api/skill-file", get(get_skill_file_api))
        .route("/api/skill-file", post(upsert_skill_file_api))
        .route("/api/skill-file", delete(delete_skill_file_api))
        // Missions
        .route("/api/missions", get(list_missions).post(create_mission))
        .route(
            "/api/missions/sessions/state",
            get(get_mission_session_state),
        )
        .route(
            "/api/missions/{id}",
            put(update_mission).delete(delete_mission),
        )
        .route("/api/missions/{id}/runs", get(list_mission_runs))
        .route("/api/missions/{id}/trigger", post(trigger_mission))
        .route(
            "/api/mission-file",
            get(get_mission_file).post(upsert_mission_file),
        )
        // Status & account
        .route("/api/status", get(get_status_api))
        .route("/api/account", get(get_account))
        .route("/api/user/name", get(get_user_name))
        .route("/api/account/login", post(post_account_login))
        .route("/api/account/callback", get(get_account_callback))
        .route("/api/account/logout", post(post_account_logout))
        .route("/api/account/checkout", post(post_account_checkout))
        // Rooms
        .route("/api/rooms", any(proxy_rooms))
        .route("/api/rooms/", any(proxy_rooms))
        .route("/api/rooms/{*path}", any(proxy_rooms))
        .route("/api/proxy/connect", post(connect_proxy_room_api))
        .route("/api/proxy/disconnect", post(disconnect_proxy_room_api))
        .route("/api/proxy/status", get(proxy_status_api))
        .route("/api/token-usage", get(token_usage_api))
        .route(
            "/api/room-config",
            get(get_room_config).post(update_room_config),
        )
        // Utilities
        .route("/api/health", get(health_handler))
        .route("/api/utils/ollama-status", get(get_ollama_status))
        // Storage browser
        .route("/api/storage/roots", get(storage_roots))
        .route("/api/storage/tree", get(storage_tree))
        .route(
            "/api/storage/file",
            get(storage_read_file)
                .put(storage_write_file)
                .delete(storage_delete_file),
        )
}

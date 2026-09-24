//! Sessions, chat turns, plans and questions (also reachable as named
//! WebRTC RPCs).

use super::Routes;
use crate::server::api::agents::{cancel_agent_run, clear_queued_messages, set_task};
use crate::server::api::permissions::{get_session_permission, update_session_permission};
use crate::server::api::sessions::{
    create_session, delete_unified_session, get_skill_session_state, list_skill_sessions,
    remove_session_api, remove_skill_session_api, rename_session_api,
};
use crate::server::chat::{
    approve_plan_handler, ask_user_response_handler, chat_handler, clear_chat_history_api,
    compact_chat_api, compact_config_api, edit_plan_handler, get_system_prompt_api,
    pending_ask_user_handler, reject_plan_handler,
};
use axum::routing::{delete, get, patch, post};

pub(super) fn routes() -> Routes {
    Routes::new()
        // Session management (create/rename/delete still via HTTP, data via page_state)
        .route("/api/sessions/all", delete(delete_unified_session))
        .route("/api/sessions", post(create_session))
        .route("/api/sessions", patch(rename_session_api))
        .route("/api/sessions", delete(remove_session_api))
        .route(
            "/api/sessions/permission",
            get(get_session_permission).patch(update_session_permission),
        )
        .route("/api/skill-sessions", get(list_skill_sessions))
        .route("/api/skill-sessions", delete(remove_skill_session_api))
        .route("/api/skill-sessions/state", get(get_skill_session_state))
        .route("/api/task", post(set_task))
        .route("/api/agent-cancel", post(cancel_agent_run))
        .route("/api/queue/clear", post(clear_queued_messages))
        // Chat & plan
        .route("/api/chat", post(chat_handler))
        .route("/api/chat/clear", post(clear_chat_history_api))
        .route("/api/chat/compact", post(compact_chat_api))
        .route("/api/chat/compact_config", post(compact_config_api))
        .route("/api/chat/system-prompt", get(get_system_prompt_api))
        .route("/api/plan/approve", post(approve_plan_handler))
        .route("/api/plan/edit", post(edit_plan_handler))
        .route("/api/plan/reject", post(reject_plan_handler))
        .route("/api/ask-user-response", post(ask_user_response_handler))
        .route("/api/pending-ask-user", get(pending_ask_user_handler))
}

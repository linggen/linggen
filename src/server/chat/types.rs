use serde::Deserialize;

pub(super) fn default_user_type() -> String {
    "owner".to_string()
}

#[derive(Deserialize)]
pub(crate) struct ChatRequest {
    pub(super) project_root: String,
    pub(super) agent_id: String,
    pub(super) message: String,
    pub(super) session_id: Option<String>,
    /// User type: "owner" (default) or "consumer" (proxy room).
    /// Injected server-side by peer.rs. Missing = owner (local HTTP requests).
    #[serde(default = "default_user_type")]
    pub(super) user_type: String,
    /// When set, this chat belongs to a mission session — persist messages
    /// under `~/.linggen/missions/{mission_id}/sessions/` instead of
    /// the project's session store.
    pub(super) mission_id: Option<String>,
    /// When set, this chat belongs to a skill session — persist messages
    /// under `~/.linggen/skills/{skill_name}/sessions/` instead of
    /// the project's session store.
    pub(super) skill_name: Option<String>,
    /// Session-level model override. Takes priority over routing.default_models.
    pub(super) model_id: Option<String>,
    /// User ID of the session creator (linggen.dev user_id).
    /// Injected by peer.rs for both owner and consumer connections.
    pub(super) user_id: Option<String>,
    #[serde(default)]
    pub(super) images: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct PlanActionRequest {
    pub(super) project_root: String,
    pub(super) agent_id: String,
    pub(super) session_id: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct EditPlanRequest {
    pub(super) project_root: String,
    pub(super) agent_id: String,
    pub(super) session_id: Option<String>,
    pub(super) text: String,
}

#[derive(Deserialize)]
pub(crate) struct ClearChatRequest {
    pub(super) project_root: String,
    pub(super) session_id: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct CompactChatRequest {
    pub(super) project_root: String,
    pub(super) session_id: Option<String>,
    pub(super) agent_id: Option<String>,
    pub(super) focus: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct SystemPromptQuery {
    pub(super) project_root: String,
    pub(super) agent_id: String,
    #[serde(default)]
    pub(super) session_id: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct AskUserResponseRequest {
    pub(super) question_id: String,
    pub(super) answers: Vec<crate::engine::tools::AskUserAnswer>,
}

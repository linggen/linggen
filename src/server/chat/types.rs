use serde::Deserialize;

pub(super) fn default_user_type() -> String {
    "owner".to_string()
}

#[derive(Clone, Deserialize)]
pub(crate) struct ChatRequest {
    pub(crate) project_root: String,
    /// The member this message is for. Empty: whoever the session's
    /// default responder is (`chat::members`) — or the `@name` it opens with.
    #[serde(default)]
    pub(crate) agent_id: String,
    pub(crate) message: String,
    pub(crate) session_id: Option<String>,
    /// User type: "owner" (default) or "consumer" (proxy room).
    /// Injected server-side by peer.rs. Missing = owner (local HTTP requests).
    #[serde(default = "default_user_type")]
    pub(crate) user_type: String,
    /// When set, this chat belongs to a mission session — persist messages
    /// under `~/.linggen/missions/{mission_id}/sessions/` instead of
    /// the project's session store.
    pub(crate) mission_id: Option<String>,
    /// When set, this chat belongs to a skill session — persist messages
    /// under `~/.linggen/skills/{skill_name}/sessions/` instead of
    /// the project's session store.
    pub(crate) skill_name: Option<String>,
    /// Session-level model override. Takes priority over routing.default_models.
    pub(crate) model_id: Option<String>,
    /// User ID of the session creator (linggen.dev user_id).
    /// Injected by peer.rs for both owner and consumer connections.
    pub(crate) user_id: Option<String>,
    #[serde(default)]
    pub(crate) images: Vec<String>,
    /// Who is speaking, when it isn't the user typing at this surface — an
    /// agent id like "yinyue" relayed from another device. Persisted as the
    /// message's from_id so every chat surface can label the bubble, and
    /// handed to the model as "[Yinyue]: …". Absent or "user" = the user.
    #[serde(default)]
    pub(crate) sender: Option<String>,
    /// The device the session's own agent is continuing the user's task from
    /// ("iphone"). Not another speaker: the row stays the user's request,
    /// read by the model and labeled on surfaces as a hand-off.
    #[serde(default)]
    pub(crate) via: Option<String>,
    /// One line in the handing-off agent's own first-person voice ('Let me check…');
    /// what surfaces show instead of the request text. Never read by the model.
    #[serde(default)]
    pub(crate) via_note: Option<String>,
    /// The surface shows follow-up buttons — ask the model for them this turn.
    #[serde(default)]
    pub(crate) followups: bool,
    /// The words were spoken aloud, at a page that holds the microphone and
    /// heard there (`shared/hold-to-talk.js`, sent through the chat bridge's
    /// `send(text, {heard: true})`). A mark on the stored row of a person's
    /// line and nothing else: it is ignored for a line sent as an agent
    /// (`sender`) or handed off (`via`), and changes no turn, permission or
    /// reply (`helpers::heard_mark` decides; not [`TurnSettings::heard`]).
    #[serde(default)]
    pub(crate) heard: bool,
    /// The id the surface gave its bubble for this message. Kept on the
    /// saved row and echoed on its `message` event, so the surface matches
    /// its bubble to the row by id — the row keeps the words after an
    /// `@name`, which differ from what was typed.
    #[serde(default)]
    pub(crate) client_id: Option<String>,
    /// The level the request was judged at off this machine
    /// (`x-linggen-level`, `api::pair::caller_level`), the turn's ceiling.
    /// Never read from the body: `chat_handler` sets it from the header;
    /// `None` is this machine's own turn.
    #[serde(skip)]
    pub(crate) level: Option<crate::engine::permission::PermissionMode>,
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
    pub(super) session_id: Option<String>,
    /// The app whose page asks (set by the page's channel): in the day's
    /// session its part of the day alone is cleared (`chat::clear_place`).
    #[serde(default)]
    pub(super) skill_name: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct CompactChatRequest {
    pub(super) project_root: String,
    pub(super) session_id: Option<String>,
    pub(super) focus: Option<String>,
}

/// Set the auto-compaction threshold + focus for a session. Persisted to
/// session.yaml and applied to the live engine when one exists — the endpoint
/// never creates an engine (extra request fields like the legacy
/// `project_root` are ignored by serde).
#[derive(Deserialize)]
pub(crate) struct CompactConfigRequest {
    pub(super) session_id: Option<String>,
    /// Override the auto-compaction trigger fraction of `context_window_tokens`.
    /// Default is 0.95. Range 0.1–0.99. Null or absent clears the override
    /// (falls back to default). Out-of-range values are clamped.
    pub(super) threshold: Option<f32>,
    /// Per-session hint passed to the summarization model whenever auto-compact
    /// fires on this session. Null/absent clears the hint (auto-compact passes
    /// `None`, matching default behavior).
    pub(super) focus: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct SystemPromptQuery {
    pub(super) project_root: String,
    pub(super) agent_id: String,
    #[serde(default)]
    pub(super) session_id: Option<String>,
    /// The app whose page asks, as a chat turn names it: the export shows
    /// the prompt and tools of that app's place.
    #[serde(default)]
    pub(super) skill_name: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct AskUserResponseRequest {
    pub(super) question_id: String,
    pub(super) answers: Vec<crate::engine::tools::AskUserAnswer>,
    /// Stamped by the RTC tunnel from the peer's identity, never trusted
    /// from a client body; a direct (LAN-gated) call is the owner's.
    #[serde(default = "default_user_type")]
    pub(super) user_type: String,
    #[serde(default)]
    pub(super) user_id: Option<String>,
}

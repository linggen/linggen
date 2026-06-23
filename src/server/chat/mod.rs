//! Chat HTTP handler pipeline.
//!
//! Stages
//! ------
//! - [`handler`]        — `chat_handler` entry point: session bootstrap,
//!                        agent locking, queueing, model resolution, then
//!                        dispatches into one of the three flows below.
//! - [`skill_dispatch`] — slash-command (`/skill`) and trigger-prefix paths
//!                        (incl. app-launcher branches and skill permission
//!                        prompt).
//! - [`structured`]     — the default agentic loop; promotes plan/PlanModeRequested
//!                        outcomes back into the plan flow.
//! - [`plan_flow`]      — plan-mode dispatch, plan execution, and
//!                        approve/reject/edit handlers.
//! - [`admin`]          — side-channel handlers (clear, compact, system-prompt
//!                        export, AskUser response/pending) that don't run the
//!                        agent loop.
//!
//! Cross-cutting helpers live in [`runtime`]: run_loop_with_tracking,
//! interrupt + AskUser bridge wiring, the thinking-channel forwarder,
//! and per-turn auto-recall.

mod admin;
mod handler;
pub(super) mod helpers;
mod plan_flow;
mod runtime;
mod skill_dispatch;
mod structured;
mod types;

pub(crate) use admin::{
    ask_user_response_handler, clear_chat_history_api, compact_chat_api, compact_config_api,
    get_system_prompt_api, pending_ask_user_handler,
};
pub(crate) use handler::{chat_handler, run_session_turn};
pub(crate) use plan_flow::{approve_plan_handler, edit_plan_handler, reject_plan_handler};

use crate::engine::agent::AgentManager;
use crate::server::{ServerEvent, ServerState};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Shared context passed to every per-turn dispatch flow (skill, trigger,
/// plan, structured, plan-execution). Built once in `chat_handler` and
/// in `approve_plan_handler` (for the resume-after-approval case).
pub(super) struct ChatRunCtx {
    pub(super) state: Arc<ServerState>,
    pub(super) manager: Arc<AgentManager>,
    pub(super) events_tx: broadcast::Sender<ServerEvent>,
    pub(super) root: PathBuf,
    pub(super) agent_id: String,
    pub(super) session_id: Option<String>,
    pub(super) clean_msg: String,
    pub(super) images: Vec<String>,
    pub(super) policy: crate::engine::session_policy::SessionPolicy,
}

/// Open a URL in the system's default browser. Used by skill app launchers
/// when the request originated from the local UI (no remote consumer).
pub(super) fn open_in_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", url])
            .spawn()?;
    }
    Ok(())
}

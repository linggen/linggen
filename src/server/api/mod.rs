//! HTTP handler modules. Each submodule owns a coherent slice of
//! `/api/*` endpoints; `server::mod` wires them into the Axum router.

pub(super) mod account;
pub(super) mod agents;
pub(super) mod auth;
pub(super) mod config;
pub(super) mod marketplace;
pub(super) mod missions;
pub(super) mod permissions;
pub(super) mod rooms;
pub(super) mod sessions;
pub(super) mod skills;
pub(super) mod status;
pub(super) mod storage;
pub(super) mod tts;
pub(super) mod workspace;
pub(super) mod yinyue;

use serde::Deserialize;
use std::path::PathBuf;

/// Shared request shape for project-scoped GET endpoints with pagination.
/// Used by `list_sessions`, `list_skill_files_api`, `list_agent_files_api`.
#[derive(Deserialize)]
pub(super) struct ProjectQuery {
    pub(super) project_root: String,
    /// Max items to return (default: all).
    #[serde(default)]
    pub(super) limit: Option<usize>,
    /// Skip this many items from the start.
    #[serde(default)]
    pub(super) offset: Option<usize>,
}

/// Expand `~` / `~/...` and resolve a project_root string into an absolute,
/// canonicalized path. Shared by agent / skill / session handlers.
pub(super) fn canonical_project_root(project_root: &str) -> PathBuf {
    let expanded = if project_root == "~" {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from(project_root))
    } else if project_root.starts_with("~/") {
        dirs::home_dir()
            .unwrap_or_default()
            .join(&project_root[2..])
    } else {
        PathBuf::from(project_root)
    };
    crate::util::resolve_path(&expanded)
}

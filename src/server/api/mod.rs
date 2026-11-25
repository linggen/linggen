//! HTTP handler modules. Each submodule owns a coherent slice of
//! `/api/*` endpoints; `server::mod` wires them into the Axum router.

pub(super) mod account;
pub(super) mod activity;
pub(super) mod app_launch;
pub(super) mod assets;
pub(super) mod agents;
pub(super) mod config;
pub(super) mod ling_mem_config;
pub(super) mod ling_mem_connect;
pub(super) mod ling_mem_primary;
pub(super) mod live;
pub(super) mod marketplace;
pub(super) mod media;
pub(super) mod memory;
pub(super) mod missions;
pub(super) mod notes;
pub(super) mod outside_apps;
/// `pub(crate)` rather than `pub(super)`: this module owns the paired-device
/// store, not just its handlers, and perception reads it to say whether a
/// device exists at all.
pub(crate) mod pair;
pub(super) mod people;
pub(super) mod permissions;
pub(super) mod provider_families;
pub(super) mod providers;
pub(super) mod quests;
pub(super) mod rooms;
pub(super) mod sessions;
pub(super) mod skill_cloud;
pub(super) mod skill_save;
pub(super) mod skill_sync;
pub(super) mod skill_tools;
pub(super) mod skills;
pub(super) mod status;
pub(super) mod storage;
/// `pub(crate)` rather than `pub(super)`: retained topics are how any reading
/// reaches the user's other devices, and perception publishes this Mac's own
/// state on one.
pub(crate) mod topic;
/// `pub(super)`: the sidecar is made once, in `server::run`, and handed to
/// her voice and her hearing.
pub(super) mod mlx_sidecar;
pub(super) mod stt;
/// The Swift helper `stt_apple` runs, carried inside `ling`.
mod apple_helper;
pub(super) mod stt_apple;
pub(super) mod stt_mlx;
pub(super) mod tts;
pub(super) mod tts_mlx;
/// `pub(crate)`: the `pet_speak` event carries a line's clauses.
pub(crate) mod tts_text;
pub(super) mod tts_wav;
pub(super) mod uses;
pub(crate) mod utils;
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
    // Without a home, `~/…` keeps its relative tail; bare `~` stays literal.
    let home = dirs::home_dir().or_else(|| project_root.starts_with("~/").then(PathBuf::new));
    let expanded = crate::util::expand_tilde(project_root.as_ref(), home.as_deref());
    crate::util::resolve_path(&expanded)
}

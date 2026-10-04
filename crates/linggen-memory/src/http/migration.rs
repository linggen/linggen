//! `/api/schema/apply` — the store's gated v1 → v2 step
//! (`doc/schema-versioning.md`, `doc/scope-index-spec.md`).
//!
//! Backs the store up, then reshapes both tables to the v2 layout: drop
//! `contexts` and `tags`, rename `hook` → `summary` and `cwd` → `scope`
//! (values carried). Stamps the sidecar `2` once no table is left on the v1
//! layout. Idempotent: a second call finds nothing to change.

use super::envelope::{ok, ApiError};
use super::state::SharedState;
use axum::extract::State;
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use std::path::{Path, PathBuf};

pub fn router() -> Router<SharedState> {
    Router::new().route("/api/schema/apply", post(apply_schema))
}

#[derive(Debug, Default, Deserialize)]
pub struct ApplySchemaRequest {
    /// Must be `true`: the step drops columns, and the caller says so.
    #[serde(default)]
    pub confirm: bool,
}

async fn apply_schema(
    State(state): State<SharedState>,
    Json(req): Json<ApplySchemaRequest>,
) -> Result<Response, ApiError> {
    if !req.confirm {
        return Err(ApiError::bad_request(
            "the v2 step drops the contexts and tags columns — pass confirm:true",
        ));
    }
    let backup_dir = {
        let data_dir = state.data_dir.clone();
        tokio::task::spawn_blocking(move || backup(&data_dir))
            .await
            .map_err(|e| ApiError::internal(e.into()))?
            .map_err(|e| ApiError::internal(e.into()))?
    };
    state.store.apply_v2_layout().await?;
    state.episodic.apply_v2_layout().await?;
    let legacy = state.store.is_v1_layout() || state.episodic.is_v1_layout();
    crate::memory::schema_version::stamp_layout(&state.data_dir, legacy)
        .map_err(ApiError::internal)?;
    Ok(ok(json!({
        "schema_version": crate::memory::schema_version::read_version(&state.data_dir),
        "backup_dir": backup_dir.to_string_lossy(),
        "applied_at": Utc::now(),
    })))
}

/// Copy the store directory and its sidecar version to
/// `memory/backups/schema-v2-<ts>/`. A plain file copy: LanceDB data files
/// are immutable once written, and a manifest copied mid-commit only leaves
/// the copy one version behind.
fn backup(data_dir: &Path) -> std::io::Result<PathBuf> {
    let mem = data_dir.join("memory");
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let dest = mem.join("backups").join(format!("schema-v2-{stamp}"));
    copy_dir(&mem.join("memory.lancedb"), &dest.join("memory.lancedb"))?;
    let version = mem.join("SCHEMA_VERSION");
    if version.exists() {
        std::fs::copy(version, dest.join("SCHEMA_VERSION"))?;
    }
    Ok(dest)
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

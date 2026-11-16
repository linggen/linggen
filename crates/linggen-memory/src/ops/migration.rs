use crate::error::MemError;
use crate::handle::Memory;
use crate::http::migration::copy_dir;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Deserialize)]
pub struct ApplySchemaRequest {
    /// Must be `true`: the step drops columns, and the caller says so.
    #[serde(default)]
    pub confirm: bool,
}

pub(in crate::ops) async fn apply_schema(state: &Memory, req: ApplySchemaRequest) -> Result<Value, MemError> {
    if !req.confirm {
        return Err(MemError::bad_request(
            "the v2 step drops the contexts and tags columns — pass confirm:true",
        ));
    }
    let backup_dir = {
        let data_dir = state.data_dir.clone();
        tokio::task::spawn_blocking(move || backup(&data_dir))
            .await
            .map_err(|e| MemError::internal(e.into()))?
            .map_err(|e| MemError::internal(e.into()))?
    };
    state.store.apply_v2_layout().await?;
    state.episodic.apply_v2_layout().await?;
    let legacy = state.store.is_v1_layout() || state.episodic.is_v1_layout();
    crate::memory::schema_version::stamp_layout(&state.data_dir, legacy)
        .map_err(MemError::internal)?;
    Ok(json!({
        "schema_version": crate::memory::schema_version::read_version(&state.data_dir),
        "backup_dir": backup_dir.to_string_lossy(),
        "applied_at": Utc::now(),
    }))
}

/// Copy the store directory and its sidecar version to
/// `memory/backups/schema-v2-<ts>/` (named by `memory::backups`, which
/// expires it after 14 days). A plain file copy: LanceDB data files
/// are immutable once written, and a manifest copied mid-commit only leaves
/// the copy one version behind.
fn backup(data_dir: &Path) -> std::io::Result<PathBuf> {
    let mem = data_dir.join("memory");
    let dest = crate::memory::backups::migration_backup_dir(
        data_dir,
        crate::memory::backups::SCHEMA_V2,
        Utc::now(),
    );
    copy_dir(&mem.join("memory.lancedb"), &dest.join("memory.lancedb"))?;
    let version = mem.join("SCHEMA_VERSION");
    if version.exists() {
        std::fs::copy(version, dest.join("SCHEMA_VERSION"))?;
    }
    Ok(dest)
}

//! `/api/schema/apply` — the store's gated v1 → v2 step
//! (`doc/schema-versioning.md`, `doc/scope-index-spec.md`).
//!
//! Backs the store up, then reshapes both tables to the v2 layout: drop
//! `contexts` and `tags`, rename `hook` → `summary` and `cwd` → `scope`
//! (values carried). Stamps the sidecar `2` once no table is left on the v1
//! layout. Idempotent: a second call finds nothing to change.

use super::state::SharedState;
use super::verb;
use crate::ops::migration::ApplySchemaRequest;
use axum::extract::{Request, State};
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use std::path::Path;

pub fn router() -> Router<SharedState> {
    Router::new().route("/api/schema/apply", post(apply_schema))
}

async fn apply_schema(State(state): State<SharedState>, req: Request) -> Response {
    verb::serve::<ApplySchemaRequest>(state, "apply_schema", req).await
}

pub(crate) fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
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

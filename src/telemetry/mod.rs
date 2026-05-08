//! Anonymous usage telemetry for the Linggen engine.
//!
//! ## What we send
//!
//! POST https://linggen.dev/api/track  (Pages Function — see linggensite repo)
//!
//! On daemon start:
//! - `install` event the first time the engine runs on this machine
//!   (installation_id newly created; payload.via from the install marker).
//! - `install` event whenever the version changes from the last recorded one
//!   (payload.via = "upgrade", payload.from_version, payload.to_version).
//! - `command` event with payload.verb = "engine.start" and payload.system_state
//!   reflecting which sibling products (Sys Doctor, ling-mem) are detected
//!   on this machine. Counts as the daily activity row for DAU.
//!
//! On every meaningful action (wired separately):
//! - `command` event with payload.verb = "skill.<name>.open" / "session.start"
//!   / etc. Verbs are stable strings; the server stores them verbatim.
//!
//! No dedicated heartbeat — DAU is derived server-side from any event row
//! (`COUNT(DISTINCT installation_id) WHERE date(created_at) = today`). The
//! engine.start event guarantees at least one row per active day.
//!
//! ## What we never send
//!
//! Chat content, file paths, prompts, model outputs, embeddings, IPs (CF
//! strips and we don't store), or any user-identifying string. The
//! installation_id is a random v4 UUID stored at `~/.linggen/installation_id`,
//! shared across all Linggen products on this machine.
//!
//! ## Disabling telemetry
//!
//! Runtime:
//!   - env: `LINGGEN_NO_TELEMETRY=1`
//!   - file: `touch ~/.linggen/no-telemetry`
//! Compile time:
//!   - `cargo build --no-default-features`
//!
//! ## OSS audit
//!
//! Every field sent is listed above. Receiver source lives in
//! `linggensite/functions/api/_lib/analytics.ts`. No third-party analytics.

#[cfg(feature = "telemetry")]
mod imp;

#[cfg(feature = "telemetry")]
pub use imp::{read_system_state, Telemetry};

#[cfg(not(feature = "telemetry"))]
pub fn read_system_state(_data_dir: &std::path::Path) -> serde_json::Value {
    serde_json::Value::Null
}

/// No-op stub used when the `telemetry` feature is disabled at compile time.
/// Keeps call sites unchanged.
#[cfg(not(feature = "telemetry"))]
#[derive(Clone)]
pub struct Telemetry;

#[cfg(not(feature = "telemetry"))]
impl Telemetry {
    pub fn new(_product: &'static str, _data_dir: &std::path::Path) -> Self {
        Self
    }
    pub fn launch(&self) {}
    pub fn command(&self, _verb: &str) {}
    pub fn command_with_payload(&self, _verb: &str, _extra: serde_json::Value) {}
}

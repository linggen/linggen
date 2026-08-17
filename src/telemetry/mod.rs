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
//!   reflecting which sibling products (Apple Shifu, ling-mem) are detected
//!   on this machine. Counts as the daily activity row for DAU.
//!
//! `payload.via` names the distribution channel this machine came through,
//! and nothing more — it is read from `~/.linggen/.linggen-install-source`,
//! a plain `key=value` file written by whichever installer ran. A label is
//! only ever set by a caller that genuinely IS that channel; nothing guesses.
//! Known values: `website` (install.sh ran with no channel label — the
//! linggen.dev one-liner and anywhere it is pasted), `plugin` (the Claude
//! Code / Codex session-start hook), `app` (a .app bundle, which runs no
//! installer and writes the marker itself, plus `app_id`), `upgrade` (not a
//! channel — a version change on an existing install), and `unknown` (no
//! marker present; pre-marker installs stay here forever). Marketplace
//! installs (`clawhub`, `skills-sh`) are labeled by the skill's
//! `scripts/bootstrap.sh`, which derives the channel from its own on-disk
//! path at run time — measured, never asserted; an unrecognized path labels
//! nothing and falls into `website`. `agent` (optional) names the host that
//! ran the install (`cc`, `codex`, `openclaw`, `linggen`, `vscode`), same
//! rule: observed from the environment or omitted. Every other key in the
//! marker file is forwarded verbatim (`installer_version`, `installed_at`,
//! `app_id`). Full design: linggensite/doc/analytics-spec.md.
//!
//! On every meaningful action (wired separately):
//! - `command` event with payload.verb = "skill.<name>.open" / "session.start"
//!   / etc. Verbs are stable strings; the server stores them verbatim.
//!
//! Daily digest (one `digest` event per completed UTC day, sent on the first
//! activity of a later day; up to 14 days of offline backlog):
//! - payload.day = "YYYY-MM-DD", payload.counts = {key: n}. High-frequency
//!   signal accumulates in a local counter file
//!   (`~/.linggen/telemetry/linggen-digest.json`) and never leaves the
//!   machine row-by-row. Every count key is from this closed list:
//!   - `engine.start` — daemon starts that day
//!   - `chat.turn_ok` — model turns that completed successfully
//!   - `update.ok` — self-update applied
//!   - `error.<stage>.<code>` — failure buckets; stage ∈ {start, model,
//!     search}, code is a coarse cause (`auth_required`, `model_not_found`,
//!     `provider_http`, `network`, `quota`, `config`, `other`). Never an
//!     error message, model name the user typed, URL, or any free text —
//!     codes are normalized to `[a-z0-9_-]` and capped at 32 chars.
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
mod digest;
#[cfg(feature = "telemetry")]
mod imp;

#[cfg(feature = "telemetry")]
pub use imp::{read_system_state, Telemetry};

/// Process-wide telemetry handle. One installation_id read, one HTTP client,
/// one digest file — shared by the server, the engine loop, and the CLI.
pub fn global() -> &'static Telemetry {
    static GLOBAL: std::sync::OnceLock<Telemetry> = std::sync::OnceLock::new();
    GLOBAL.get_or_init(|| Telemetry::new("linggen", crate::paths::linggen_home()))
}

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
    pub fn bump(&self, _key: &str) {}
    pub fn error(&self, _stage: &str, _code: &str) {}
}

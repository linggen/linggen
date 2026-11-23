//! The Linggen engine: everything the `ling` binary runs but its HTTP/RTC
//! server, its CLI and `main` (doc/architecture-review.md § 2, step 3.2).

pub mod account;
pub mod agent_ids;
pub mod assets;
pub mod cannot_run;
pub mod config;
pub mod credentials;
pub mod engine;
pub mod extensions;
mod front;
pub mod hf_fetch;
pub mod hf_mirror;
pub mod js_runtime;
pub mod language;
pub mod logging;
pub mod mcp_client;
pub mod memory_host;
pub mod message;
pub mod mirror;
pub mod notes;
pub mod paths;
pub mod permission_mode;
pub mod perception;
pub mod pictures;
pub mod project_root;
pub mod prompts;
pub mod provider;
pub mod reach;
pub mod runtime;
pub mod senses;
pub mod state_fs;
pub mod telemetry;
pub mod util;

//! Shared substrate for extension runtimes — skills (interactive) and
//! missions (scheduled headless). Both are markdown-frontmatter artifacts
//! the engine discovers, loads, and runs. The runtime contracts diverge
//! (interactive vs autonomous), but the on-disk shape and the parsing /
//! script-launch / tool-scope helpers are identical.
//!
//! See `doc/skill-spec.md` and `doc/mission-spec.md` for each runtime;
//! the `engine/permission/manifest.rs` module owns the permission grammar
//! both share.

pub mod frontmatter;
pub mod marketplace;
pub mod missions;
pub mod scope;
pub mod script;
pub mod skills;

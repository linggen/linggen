//! Skill runtime records + lookup contract.
//!
//! - `record` — the `Skill` runtime record (frontmatter + manifest +
//!   permission + tool defs). Pure data type the engine reads.
//! - `params` — session parameters: values a skill session is bound to.
//! - `tools` — running a declared shell tool outside a turn (the page
//!   door and the companion's `AppTool`).
//! - `outside` — what an agent outside Linggen, at the MCP door, is
//!   offered of an app and may run.
//! - `registry` — `SkillRegistry` trait, the engine's spec lookup
//!   contract for skills.
//!
//! Mirrors `engine::agent` (`record` + `registry` + concrete
//! `RunStore`) and `engine::mission` (`record` + `registry` +
//! `MissionRunStore`). Disk loading lives in `extensions::skills`;
//! that module's `SkillLoader` is the production impl of
//! `SkillRegistry`.

pub mod judge;
pub mod outside;
pub mod params;
pub mod reach;
pub mod record;
pub mod registry;
pub mod tools;

#[cfg(test)]
mod door_tests;

pub use record::{
    AppConfig, CloudConfig, NoticeDecl, NoticeTier, QuestsConfig, QueueMode, Skill, SkillSource,
    SyncConfig,
};
pub use registry::SkillRegistry;

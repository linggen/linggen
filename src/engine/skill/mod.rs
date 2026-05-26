//! Skill runtime records + lookup contract.
//!
//! - `record` — the `Skill` runtime record (frontmatter + manifest +
//!   permission + tool defs). Pure data type the engine reads.
//! - `registry` — `SkillRegistry` trait, the engine's spec lookup
//!   contract for skills.
//!
//! Mirrors `engine::agent` (`record` + `registry` + concrete
//! `RunStore`) and `engine::mission` (`record` + `registry` +
//! `MissionRunStore`). Disk loading lives in `extensions::skills`;
//! that module's `SkillLoader` is the production impl of
//! `SkillRegistry`.

pub mod record;
pub mod registry;

pub use record::{AppConfig, CapabilityImpl, Skill, SkillSource};
pub use registry::SkillRegistry;

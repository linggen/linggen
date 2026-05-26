//! Engine's contract for looking up skills.
//!
//! Lets the engine query for installed skills (by name, by metadata
//! listing) without knowing how they're loaded or stored.
//! `extensions::skills::SkillLoader` is the production implementer;
//! tests can stub against a smaller in-memory impl.
//!
//! Returns `engine::skill::Skill` records — owned shapes the engine
//! reads against.

use crate::engine::skill::Skill;
use async_trait::async_trait;

#[async_trait]
pub trait SkillRegistry: Send + Sync {
    /// Look up a skill by exact name. None if no skill with that name
    /// is installed at the time of the call.
    async fn get(&self, name: &str) -> Option<Skill>;

    /// `(name, description)` for every installed skill exposed to the
    /// model — i.e. excluding skills with `disable_model_invocation: true`.
    /// Used to populate the system prompt's "available skills" listing.
    async fn list_metadata(&self) -> Vec<(String, String)>;
}

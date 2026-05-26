//! Engine's contract for looking up skills.
//!
//! Lets the engine query for installed skills (by name, by capability,
//! by metadata listing) without knowing how they're loaded or stored.
//! `extensions::skills::SkillLoader` is the production implementer;
//! tests can stub against a smaller in-memory impl.
//!
//! Returns `engine::skill::Skill` records — owned shapes the engine
//! reads against. The trait deliberately surfaces full `Skill` values
//! rather than per-call slim views because every engine consumer
//! (capability dispatch, activation, prompt assembly) needs different
//! fields. Cloning the record is cheap (a few Strings + a HashMap);
//! the alternative — N narrow trait methods — would scatter the
//! contract.

use crate::engine::skill::Skill;
use async_trait::async_trait;

#[async_trait]
pub trait SkillRegistry: Send + Sync {
    /// Look up a skill by exact name. None if no skill with that name
    /// is installed at the time of the call.
    async fn get(&self, name: &str) -> Option<Skill>;

    /// Return the active provider for the given capability, if any.
    /// "Active" is the registry's choice — typically the first installed
    /// skill that `provides:` the capability and has an `implements:`
    /// block for it. See `extensions::skills::SkillLoader::active_provider`.
    async fn active_provider(&self, capability: &str) -> Option<Skill>;

    /// `(name, description)` for every installed skill exposed to the
    /// model — i.e. excluding skills with `disable_model_invocation: true`.
    /// Used to populate the system prompt's "available skills" listing.
    async fn list_metadata(&self) -> Vec<(String, String)>;
}

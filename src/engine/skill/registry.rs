//! Engine's contract for looking up skills.
//!
//! Lets the engine query for installed skills (by name, by listing, by a
//! typed trigger) without knowing how they're loaded or stored. The
//! engine holds an `Arc<dyn SkillRegistry>`; it never names the loader.
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
    async fn get_skill(&self, name: &str) -> Option<Skill>;

    /// Re-read one skill from disk (so an edited SKILL.md is picked up)
    /// and return it; falls back to the cached copy when the file is gone.
    async fn reload_one(&self, name: &str) -> Option<Skill>;

    /// Every installed skill.
    async fn list_skills(&self) -> Vec<Skill>;

    /// A message that invokes a skill by its trigger → `(skill, rest)`.
    async fn match_trigger(&self, input: &str) -> Option<(String, String)>;

    /// `(name, description)` for every installed skill exposed to the
    /// model — i.e. excluding skills with `disable_model_invocation: true`.
    /// Used to populate the system prompt's "available skills" listing.
    /// (name, description, is_app) — is_app marks skills with an `app` launcher,
    /// so prompts can flag which skills are routable apps.
    async fn list_metadata(&self) -> Vec<(String, String, bool)>;
}

/// An `Arc` of a registry is a registry — so `&manager.skills` passes
/// wherever a `&dyn SkillRegistry` is taken.
#[async_trait]
impl<T: SkillRegistry + ?Sized> SkillRegistry for std::sync::Arc<T> {
    async fn get_skill(&self, name: &str) -> Option<Skill> {
        (**self).get_skill(name).await
    }
    async fn reload_one(&self, name: &str) -> Option<Skill> {
        (**self).reload_one(name).await
    }
    async fn list_skills(&self) -> Vec<Skill> {
        (**self).list_skills().await
    }
    async fn match_trigger(&self, input: &str) -> Option<(String, String)> {
        (**self).match_trigger(input).await
    }
    async fn list_metadata(&self) -> Vec<(String, String, bool)> {
        (**self).list_metadata().await
    }
}

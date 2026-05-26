//! Engine's contract for looking up agents.
//!
//! Lets the engine query for installed agent specs (by id, by listing)
//! without knowing how they're loaded or stored.
//! `extensions::agents::AgentLoader` is the production implementer;
//! tests can stub against a smaller in-memory impl.
//!
//! Returns `engine::agent::AgentSpecFile` records — owned shapes the
//! engine reads against (`spec` + `system_prompt` + disk path). The
//! agent registry is project-scoped because agents are layered:
//! `~/.linggen/agents/` (global, low priority) merged with
//! `<project>/agents/` (project, high priority). See
//! `extensions::agents` for the layering rule.

use crate::engine::agent::record::AgentSpecFile;
use anyhow::Result;
use async_trait::async_trait;
use std::path::Path;

#[async_trait]
pub trait AgentRegistry: Send + Sync {
    /// All agent specs visible to `project_root`, deduped by `agent_id`
    /// with project specs overriding globals. Sorted by `agent_id`.
    async fn list(&self, project_root: &Path) -> Result<Vec<AgentSpecFile>>;

    /// Look up a single agent by id, applying the same layering as
    /// `list`. None if no matching spec is installed.
    async fn find(&self, project_root: &Path, agent_id: &str) -> Result<Option<AgentSpecFile>>;
}

//! Tool-scope helper shared by skills and missions.
//!
//! `allowed-tools` in extension frontmatter restricts which engine tools
//! the agent may invoke during this extension's run. An empty list means
//! "no restriction" (inherit the session's full tool set); a non-empty
//! list is converted to a HashSet the engine checks against on each tool
//! call.

use std::collections::HashSet;

/// Translate an `allowed-tools` list into the engine's restriction set.
/// Empty input → `None` (unrestricted); non-empty → `Some(set)`.
///
/// Used by mission dispatch (`mission_allowed_tools`) and by skill
/// activation when a skill declares `allowed-tools` for its child agent.
pub fn compute_tool_scope(allowed_tools: &[String]) -> Option<HashSet<String>> {
    if allowed_tools.is_empty() {
        None
    } else {
        Some(allowed_tools.iter().cloned().collect())
    }
}

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
///
/// A whole MCP server may be named instead of each of its tools — `mcp__memory`
/// or `mcp__memory__*`, the shape the ecosystem's allow rules already use — and
/// is expanded here into the tools that server actually advertises. The
/// expansion happens at build time on purpose: these sets are **intersected**
/// with each other downstream (mission ∩ consumer ∩ skill), and intersecting a
/// pattern with a literal name yields nothing. Every set the engine compares is
/// therefore literal, and a declaration naming a server keeps working when that
/// server's tool list changes.
pub fn compute_tool_scope(allowed_tools: &[String]) -> Option<HashSet<String>> {
    if allowed_tools.is_empty() {
        return None;
    }
    Some(
        allowed_tools
            .iter()
            .flat_map(|e| expand_declaration(e))
            .collect(),
    )
}

/// One `allowed-tools` / `tools:` entry, expanded to the names the engine will
/// compare against: a whole-server MCP entry becomes that server's advertised
/// tools, anything else stays itself.
///
/// Shared with prompt assembly on purpose. A declaration means the same thing
/// wherever it is written down, and it was read in two places — the scope set
/// expanded `mcp__memory` while the model's tool list did not, so a skill that
/// declared the server was offered a name no server answers to.
pub fn expand_declaration(entry: &str) -> Vec<String> {
    match server_wildcard(entry) {
        Some(server) => expand_server(server),
        None => vec![entry.to_string()],
    }
}

/// The server named by a whole-server entry, if this is one. `mcp__memory` and
/// `mcp__memory__*` both name `memory`; `mcp__memory__memory_add` names no
/// server, it names a tool.
fn server_wildcard(entry: &str) -> Option<&str> {
    let rest = entry.strip_prefix("mcp__")?;
    if rest.is_empty() {
        return None;
    }
    match rest.split_once("__") {
        None => Some(rest),
        Some((server, "*")) => Some(server),
        Some(_) => None,
    }
}

/// Every tool one server currently advertises, qualified.
///
/// A server that isn't connected expands to nothing, which narrows the scope to
/// what exists rather than granting a name the model can't call anyway.
fn expand_server(server: &str) -> Vec<String> {
    crate::mcp_client::registry()
        .advertised()
        .into_iter()
        .filter(|t| t.server == server)
        .map(|t| t.qualified)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_list_is_no_restriction_at_all() {
        assert!(compute_tool_scope(&[]).is_none());
    }

    #[test]
    fn ordinary_names_pass_through_untouched() {
        let scope = compute_tool_scope(&["Read".to_string(), "Grep".to_string()]).unwrap();
        assert!(scope.contains("Read"));
        assert!(scope.contains("Grep"));
        assert_eq!(scope.len(), 2);
    }

    /// A tool name is not a server name — the distinction that keeps
    /// `mcp__memory__memory_add` from silently granting the whole server.
    #[test]
    fn only_a_whole_server_entry_expands() {
        assert_eq!(server_wildcard("mcp__memory"), Some("memory"));
        assert_eq!(server_wildcard("mcp__memory__*"), Some("memory"));
        assert_eq!(server_wildcard("mcp__memory__memory_add"), None);
        assert_eq!(server_wildcard("Read"), None);
        assert_eq!(server_wildcard("mcp__"), None);
    }

    /// With no server connected the entry contributes nothing — and must not
    /// leave the pattern itself in the set, where it would be compared against
    /// real tool names and never match.
    #[test]
    fn an_unconnected_server_grants_nothing_rather_than_a_pattern() {
        let scope = compute_tool_scope(&["Read".to_string(), "mcp__memory".to_string()]).unwrap();
        assert!(scope.contains("Read"));
        assert!(!scope.contains("mcp__memory"));
    }
}

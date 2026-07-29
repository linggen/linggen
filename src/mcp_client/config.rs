//! What a user writes down to reach an MCP server.
//!
//! The field names are not ours. MCP standardises the protocol, not where a
//! host keeps its server list — Claude Code uses `.mcp.json` plus
//! `~/.claude.json`, Codex uses `[mcp_servers]`, Cursor uses
//! `.cursor/mcp.json`. What every one of them agrees on is the *shape* of an
//! entry, so we mirror it exactly: an entry copies across from any of them
//! without being rewritten, and a repo's own `.mcp.json` can be read as-is.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One server, keyed by the name it is listed under.
///
/// Which transport to use is derived, not declared: `command` means stdio,
/// `url` means streamable HTTP. That is how the ecosystem's files read, and
/// asking the user to state a `type` they already implied is a knob with no
/// decision behind it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpServerConfig {
    /// Executable for a stdio server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,

    /// Extra environment for a stdio child. Inherited env still applies.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,

    /// Endpoint for a streamable-HTTP server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// Headers for an HTTP server — where a device token or an API key goes.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,

    /// Accepted and ignored. Present because the ecosystem's files carry it
    /// and a copied entry must not fail to parse over a field we derive.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,

    /// Off without deleting the entry — so a server can be parked without
    /// losing how it was configured.
    #[serde(default = "crate::config::default_true")]
    pub enabled: bool,
}

/// Hand-written, NOT derived. `#[serde(default = ...)]` only applies when
/// deserializing, so a derived `Default` would give `enabled: false` and
/// every config built in Rust would be silently switched off while the same
/// entry read from a file was on. Two defaults that disagree is the whole
/// class of bug this arc exists to remove.
impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            command: None,
            args: Vec::new(),
            env: BTreeMap::new(),
            url: None,
            headers: BTreeMap::new(),
            kind: None,
            enabled: true,
        }
    }
}

/// How to reach a server, once derived.
#[derive(Debug, Clone, PartialEq)]
pub enum Transport {
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
    },
    Http {
        url: String,
        headers: BTreeMap<String, String>,
    },
}

impl McpServerConfig {
    /// Derive the transport, or say why the entry can't be used.
    ///
    /// An entry with neither `command` nor `url` is not a server; an entry
    /// with both is ambiguous and we refuse to guess rather than silently
    /// picking one and leaving the user wondering which.
    pub fn transport(&self) -> Result<Transport, String> {
        match (self.command.as_deref(), self.url.as_deref()) {
            (Some(c), None) if !c.is_empty() => Ok(Transport::Stdio {
                command: c.to_string(),
                args: self.args.clone(),
                env: self.env.clone(),
            }),
            (None, Some(u)) if !u.is_empty() => Ok(Transport::Http {
                url: u.to_string(),
                headers: self.headers.clone(),
            }),
            (Some(_), Some(_)) => {
                Err("has both `command` and `url` — pick one".into())
            }
            _ => Err("needs either `command` (stdio) or `url` (http)".into()),
        }
    }
}

/// The `mcpServers` object as every host in the ecosystem writes it. Used to
/// read a project's `.mcp.json` verbatim.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpServersFile {
    #[serde(rename = "mcpServers", default)]
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
}

/// Which file an entry came from, so the UI can show a project-scoped
/// server as read-only and say where it is from. A server the engine loads
/// but never shows would be hidden state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// `linggen.runtime.toml` — the user's own, editable.
    User,
    /// The workspace's `.mcp.json` — the repo's, read-only here.
    Project,
}

/// Read a workspace's own `.mcp.json`, if it has one.
///
/// This is why the field names are the ecosystem's: a repo already set up
/// for Claude Code or Cursor works in Linggen with no configuration at all.
/// Missing file is the normal case, not an error; a malformed one is
/// reported rather than silently ignored, because a repo that meant to
/// offer tools and doesn't is worth a line.
pub fn read_project_file(workspace_root: &std::path::Path) -> (BTreeMap<String, McpServerConfig>, Option<String>) {
    let path = workspace_root.join(".mcp.json");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return (BTreeMap::new(), None);
    };
    match serde_json::from_str::<McpServersFile>(&raw) {
        Ok(f) => (f.mcp_servers, None),
        Err(e) => (BTreeMap::new(), Some(format!("{}: {e}", path.display()))),
    }
}

/// Merge project entries under the user's. A name defined in both is the
/// USER's — their own config is the one they can see and edit in Settings,
/// so it must not be silently overridden by a file inside a repo they
/// cloned.
pub fn merge_scopes(
    user: &BTreeMap<String, McpServerConfig>,
    project: &BTreeMap<String, McpServerConfig>,
) -> Vec<(String, McpServerConfig, Scope)> {
    let mut out: Vec<(String, McpServerConfig, Scope)> = user
        .iter()
        .map(|(k, v)| (k.clone(), v.clone(), Scope::User))
        .collect();
    for (k, v) in project {
        if !user.contains_key(k) {
            out.push((k.clone(), v.clone(), Scope::Project));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claude_code_entry_parses_unchanged() {
        // Copied from the shape CC writes, `type` and all.
        let f: McpServersFile = serde_json::from_str(
            r#"{"mcpServers":{"linggen":{"type":"http","url":"http://127.0.0.1:9527/mcp"}}}"#,
        )
        .unwrap();
        let s = &f.mcp_servers["linggen"];
        assert_eq!(
            s.transport().unwrap(),
            Transport::Http {
                url: "http://127.0.0.1:9527/mcp".into(),
                headers: BTreeMap::new()
            }
        );
        // Enabled by default: a copied entry the user didn't annotate is on.
        assert!(s.enabled);
    }

    #[test]
    fn a_stdio_entry_parses_unchanged() {
        let f: McpServersFile = serde_json::from_str(
            r#"{"mcpServers":{"gh":{"command":"npx","args":["-y","gh-mcp"],
                 "env":{"GH_TOKEN":"x"}}}}"#,
        )
        .unwrap();
        match f.mcp_servers["gh"].transport().unwrap() {
            Transport::Stdio { command, args, env } => {
                assert_eq!(command, "npx");
                assert_eq!(args, ["-y", "gh-mcp"]);
                assert_eq!(env["GH_TOKEN"], "x");
            }
            other => panic!("expected stdio, got {other:?}"),
        }
    }

    /// The derived default said `enabled: false` while the same entry parsed
    /// from a file said true — so a server configured in Rust never dialled.
    #[test]
    fn the_rust_default_and_the_file_default_agree() {
        let from_rust = McpServerConfig::default().enabled;
        let from_file: McpServerConfig = serde_json::from_str("{}").unwrap();
        assert!(from_rust, "a server built in Rust must default to enabled");
        assert_eq!(from_rust, from_file.enabled);
    }

    #[test]
    fn a_repo_set_up_for_another_host_just_works() {
        let dir = std::env::temp_dir().join(format!("mcp_proj_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(".mcp.json"),
            r#"{"mcpServers":{"gh":{"command":"npx","args":["-y","gh-mcp"]}}}"#,
        )
        .unwrap();
        let (found, err) = read_project_file(&dir);
        assert!(err.is_none());
        assert!(found.contains_key("gh"));
        // Enabled, because the repo listed it and didn't say otherwise.
        assert!(found["gh"].enabled);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_project_file_is_normal_but_a_broken_one_is_reported() {
        let dir = std::env::temp_dir().join(format!("mcp_none_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (found, err) = read_project_file(&dir);
        assert!(found.is_empty() && err.is_none(), "a repo without one is ordinary");

        std::fs::write(dir.join(".mcp.json"), "{ not json").unwrap();
        let (_, err) = read_project_file(&dir);
        assert!(err.is_some(), "a repo that meant to offer tools deserves a line");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_users_own_entry_wins_over_a_cloned_repos() {
        let mut user = BTreeMap::new();
        user.insert("gh".to_string(), McpServerConfig { url: Some("http://mine".into()), ..Default::default() });
        let mut project = BTreeMap::new();
        project.insert("gh".to_string(), McpServerConfig { command: Some("theirs".into()), ..Default::default() });
        project.insert("extra".to_string(), McpServerConfig { command: Some("x".into()), ..Default::default() });

        let merged = merge_scopes(&user, &project);
        let gh = merged.iter().find(|(n, _, _)| n == "gh").unwrap();
        assert_eq!(gh.2, Scope::User);
        assert_eq!(gh.1.url.as_deref(), Some("http://mine"));
        // …and the repo's other server still comes along.
        assert_eq!(merged.iter().find(|(n, _, _)| n == "extra").unwrap().2, Scope::Project);
    }

    #[test]
    fn an_unusable_entry_says_why_rather_than_guessing() {
        let neither = McpServerConfig::default();
        assert!(neither.transport().unwrap_err().contains("either"));

        let both = McpServerConfig {
            command: Some("x".into()),
            url: Some("http://y".into()),
            ..Default::default()
        };
        assert!(both.transport().unwrap_err().contains("both"));
    }
}

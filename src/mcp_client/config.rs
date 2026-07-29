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
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
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

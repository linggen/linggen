//! What a user writes down to reach an MCP server.
//!
//! The field names are not ours. MCP standardises the protocol, not where a
//! host keeps its server list — Claude Code uses `.mcp.json` plus
//! `~/.claude.json`, Codex uses `[mcp_servers]`, Cursor uses
//! `.cursor/mcp.json`. What every one of them agrees on is the *shape* of an
//! entry, so we mirror it exactly: an entry copies across from any of them
//! without being rewritten.
//!
//! **One scope.** Every server Linggen connects to is the user's own, listed
//! in `[mcp_servers]`. There is no project scope: the engine does not read a
//! repo's `.mcp.json`. That is a deliberate removal, not a gap — a stdio
//! entry is `command` + `args`, so honouring a file inside a cloned repo
//! means launching whatever it names. Claude Code can offer that because it
//! gates it behind an approval prompt and refuses to let a repo approve
//! itself; a daemon with one workspace root resolved at boot has nowhere
//! sound to put that prompt.

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
    /// losing how it was configured. This, not a permission rule, is how you
    /// turn a server off: `gated` below decides whether its tools *prompt*,
    /// never whether they exist.
    #[serde(default = "crate::config::default_true")]
    pub enabled: bool,

    /// Whether this server's tools go through the permission gate.
    ///
    /// **Defaults to `true`** — an added server is a third-party process that
    /// can write files and call APIs, which is what `permission-spec.md`
    /// exists for. `false` is the direct analogue of a Claude Code allow rule
    /// covering a whole server: the user, who is the host here, deciding they
    /// trust it. That is the only direction trust flows. A *server* can never
    /// clear its own gate — MCP's `readOnlyHint` is an advisory hint from the
    /// thing being gated, and must never widen access.
    ///
    /// The built-in memory server ships `false`, which is why nothing in the
    /// permission code needs to know ling-mem's name. If gating memory were a
    /// hardcoded exemption instead of a declared one it would be invisible;
    /// as a field it round-trips through the config, the API, and the tab.
    #[serde(default = "crate::config::default_true")]
    pub gated: bool,
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
            gated: true,
        }
    }
}

/// The name the built-in memory server is listed under. A user entry with
/// this name wins — see [`with_builtin`].
pub const BUILTIN_MEMORY: &str = "memory";

/// The built-in memory server, added to whatever the user configured.
///
/// Memory is not a third party: it ships with the product, so the question
/// permission exists to answer — *do I trust this server?* — was settled at
/// install. Gating it would also break the thing it is for, since a chat-mode
/// session that cannot remember defeats the feature, and auto-recall already
/// runs before the model with no tool call to gate.
///
/// `gated: false` is carried on the entry rather than special-cased in the
/// permission code, so the exemption is visible wherever servers are and the
/// engine never learns this server's name.
pub fn builtin_memory(url: &str) -> McpServerConfig {
    McpServerConfig {
        url: Some(format!("{}/mcp", url.trim_end_matches('/'))),
        gated: false,
        ..Default::default()
    }
}

/// Merge the built-in memory server into the user's list.
///
/// A user entry named `memory` wins outright — including a disabled one,
/// which is how memory is turned off. Silently re-adding it would make
/// `enabled = false` a lie, and a switch that doesn't switch is worse than
/// no switch.
pub fn with_builtin(
    configured: &BTreeMap<String, McpServerConfig>,
    memory_url: &str,
) -> BTreeMap<String, McpServerConfig> {
    let mut out = configured.clone();
    out.entry(BUILTIN_MEMORY.to_string())
        .or_insert_with(|| builtin_memory(memory_url));
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claude_code_entry_parses_unchanged() {
        // Copied from the shape CC writes, `type` and all. The file it came
        // from is not read; the *entry* still transliterates, which is the
        // property worth keeping.
        let s: McpServerConfig = serde_json::from_str(
            r#"{"type":"http","url":"http://127.0.0.1:9527/mcp"}"#,
        )
        .unwrap();
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
        let s: McpServerConfig = serde_json::from_str(
            r#"{"command":"npx","args":["-y","gh-mcp"],"env":{"GH_TOKEN":"x"}}"#,
        )
        .unwrap();
        match s.transport().unwrap() {
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

    /// The safe default is the whole point: an entry the user pasted from
    /// somewhere is a third-party process until they say otherwise.
    #[test]
    fn an_added_server_is_gated_unless_it_says_otherwise() {
        let pasted: McpServerConfig =
            serde_json::from_str(r#"{"command":"npx","args":["-y","gh-mcp"]}"#).unwrap();
        assert!(pasted.gated);
        assert!(McpServerConfig::default().gated);

        // And a user who trusts one can say so — the host-side de-escalation
        // that CC spells as an allow rule.
        let trusted: McpServerConfig =
            serde_json::from_str(r#"{"url":"http://x/mcp","gated":false}"#).unwrap();
        assert!(!trusted.gated);
    }

    #[test]
    fn the_builtin_memory_server_is_added_ungated() {
        let none = BTreeMap::new();
        let merged = with_builtin(&none, "http://127.0.0.1:9528");
        let mem = &merged[BUILTIN_MEMORY];
        assert_eq!(mem.url.as_deref(), Some("http://127.0.0.1:9528/mcp"));
        assert!(!mem.gated, "memory ships ungated");
        assert!(mem.enabled);

        // A trailing slash on the configured base must not double up.
        let slashed = with_builtin(&none, "http://127.0.0.1:9528/");
        assert_eq!(slashed[BUILTIN_MEMORY].url.as_deref(), Some("http://127.0.0.1:9528/mcp"));
    }

    /// Silently re-adding it would make `enabled = false` a lie, and a switch
    /// that doesn't switch is worse than no switch.
    #[test]
    fn the_users_own_memory_entry_wins_including_a_disabled_one() {
        let mut user = BTreeMap::new();
        user.insert(
            BUILTIN_MEMORY.to_string(),
            McpServerConfig { enabled: false, url: Some("http://elsewhere/mcp".into()), ..Default::default() },
        );
        let merged = with_builtin(&user, "http://127.0.0.1:9528");
        assert!(!merged[BUILTIN_MEMORY].enabled, "turning memory off must stick");
        assert_eq!(merged[BUILTIN_MEMORY].url.as_deref(), Some("http://elsewhere/mcp"));
        assert_eq!(merged.len(), 1);
    }

    #[test]
    fn other_servers_survive_the_merge_and_keep_their_gate() {
        let mut user = BTreeMap::new();
        user.insert("gh".to_string(), McpServerConfig { command: Some("npx".into()), ..Default::default() });
        let merged = with_builtin(&user, "http://127.0.0.1:9528");
        assert_eq!(merged.len(), 2);
        assert!(merged["gh"].gated);
        assert!(!merged[BUILTIN_MEMORY].gated);
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

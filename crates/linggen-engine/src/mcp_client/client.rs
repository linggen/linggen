//! One connected server: the handshake, what it offers, and calling it.

use super::config::{McpServerConfig, Transport as TransportKind};
use super::transport::{HttpTransport, StdioTransport, Transport};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

/// What this engine tells a server about itself.
const CLIENT_NAME: &str = "linggen";

/// A bound on `tools/list` pages, so a server repeating its cursor cannot
/// hold discovery forever.
const MAX_TOOL_PAGES: usize = 50;

/// One tool a server offers.
#[derive(Debug, Clone, PartialEq)]
pub struct McpTool {
    /// The name the server knows it by — what goes back on `tools/call`.
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

pub struct McpClient {
    transport: Arc<dyn Transport>,
    next_id: AtomicI64,
    /// A server's own guidance, from `initialize`. This is how a server ships
    /// its doctrine to the agent instead of the host hand-copying it — which
    /// is the whole reason phase 2 collapses the memory protocol's duplicated
    /// surfaces.
    instructions: Option<String>,
}

impl McpClient {
    /// Connect and handshake. Errors here mean "this server is unavailable",
    /// never "the turn failed" — the caller reports and carries on.
    pub async fn connect(cfg: &McpServerConfig) -> Result<Self> {
        let transport: Arc<dyn Transport> = match cfg.transport().map_err(anyhow::Error::msg)? {
            TransportKind::Stdio { command, args, env } => {
                Arc::new(StdioTransport::spawn(&command, &args, &env).await?)
            }
            TransportKind::Http { url, headers } => Arc::new(HttpTransport::new(url, headers)?),
        };
        let mut client = Self {
            transport,
            next_id: AtomicI64::new(1),
            instructions: None,
        };
        client.initialize().await?;
        Ok(client)
    }

    fn id(&self) -> i64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn initialize(&mut self) -> Result<()> {
        let result = self
            .transport
            .request(json!({
                "jsonrpc": "2.0",
                "id": self.id(),
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": { "name": CLIENT_NAME, "version": env!("CARGO_PKG_VERSION") },
                },
            }))
            .await
            .context("MCP initialize")?;

        self.instructions = result
            .get("instructions")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string);

        // Required by the spec after a successful initialize. A stateless
        // server ignores it; one that wants it would otherwise refuse
        // everything after the handshake.
        let _ = self
            .transport
            .notify(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .await;
        Ok(())
    }

    /// The server's own doctrine, if it shipped any.
    pub fn instructions(&self) -> Option<&str> {
        self.instructions.as_deref()
    }

    /// What this server offers, every page of it (`nextCursor`). An empty
    /// list is a legal answer.
    pub async fn list_tools(&self) -> Result<Vec<McpTool>> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_TOOL_PAGES {
            let mut req = json!({ "jsonrpc": "2.0", "id": self.id(), "method": "tools/list" });
            if let Some(c) = cursor.take() {
                req["params"] = json!({ "cursor": c });
            }
            let result = self.transport.request(req).await.context("MCP tools/list")?;
            if let Some(page) = result.get("tools").and_then(Value::as_array) {
                tools.extend(page.iter().cloned());
            }
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty())
                .map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        Ok(tools
            .into_iter()
            .filter_map(|t| {
                let name = t.get("name")?.as_str()?.to_string();
                Some(McpTool {
                    description: t
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    // A tool with no schema takes no arguments; the model
                    // still needs an object to fill in.
                    input_schema: t
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
                    name,
                })
            })
            .collect())
    }

    /// Call a tool and return what it produced, flattened to text.
    ///
    /// MCP answers with a list of content blocks, and a tool returning JSON
    /// puts it in a `text` block as a *string* — so a caller that wants
    /// structured data parses this once more. That double encoding is the
    /// protocol's, not ours.
    pub async fn call_tool(&self, name: &str, args: Value) -> Result<String> {
        let result = self
            .transport
            .request(json!({
                "jsonrpc": "2.0",
                "id": self.id(),
                "method": "tools/call",
                "params": { "name": name, "arguments": args },
            }))
            .await
            .with_context(|| format!("MCP tools/call {name}"))?;

        // `isError` is the server saying the TOOL failed while the CALL
        // succeeded. Surface it as text: the model is what decides what to do
        // about a tool that refused, exactly as with our own tools.
        let text = content_text(&result);
        if result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Ok(format!("error: {text}"));
        }
        Ok(text)
    }
}

/// Flatten MCP content blocks into what a tool result should read like.
fn content_text(result: &Value) -> String {
    let Some(blocks) = result.get("content").and_then(Value::as_array) else {
        // Some servers answer with structured content and no blocks.
        return result
            .get("structuredContent")
            .map(|v| v.to_string())
            .unwrap_or_default();
    };
    blocks
        .iter()
        .filter_map(|b| match b.get("type").and_then(Value::as_str) {
            Some("text") => b.get("text").and_then(Value::as_str).map(str::to_string),
            // Anything not text (images, embedded resources) is named rather
            // than dropped, so a truncated-looking result is explicable.
            Some(other) => Some(format!("[{other} content]")),
            None => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_blocks_flatten_and_others_are_named_not_dropped() {
        let r = json!({"content":[
            {"type":"text","text":"first"},
            {"type":"image","data":"…"},
            {"type":"text","text":"second"}
        ]});
        assert_eq!(content_text(&r), "first\n[image content]\nsecond");
    }

    #[test]
    fn structured_content_survives_when_there_are_no_blocks() {
        let r = json!({"structuredContent":{"rows":2}});
        assert_eq!(content_text(&r), r#"{"rows":2}"#);
    }

    /// A server that answers `tools/list` in pages: three tools over two.
    struct Paged;

    #[async_trait::async_trait]
    impl Transport for Paged {
        async fn request(&self, body: Value) -> Result<Value> {
            let tool = |n: &str| json!({"name": n, "inputSchema": {"type": "object"}});
            Ok(match body.pointer("/params/cursor").and_then(Value::as_str) {
                None => json!({"tools": [tool("a"), tool("b")], "nextCursor": "p2"}),
                Some("p2") => json!({"tools": [tool("c")]}),
                Some(other) => anyhow::bail!("unknown cursor {other}"),
            })
        }
        async fn notify(&self, _body: Value) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn discovery_reads_every_page() {
        let client = McpClient {
            transport: Arc::new(Paged),
            next_id: AtomicI64::new(1),
            instructions: None,
        };
        let names: Vec<String> = client.list_tools().await.unwrap().into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["a", "b", "c"]);
    }

    #[test]
    fn no_content_at_all_is_empty_not_a_panic() {
        assert_eq!(content_text(&json!({})), "");
    }

    /// The whole stack against a real server — the hosted memory on a temp
    /// store: handshake, discovery, call.
    #[tokio::test]
    async fn talks_to_a_memory_server() {
        let (url, _store) = crate::mcp_client::test_memory::mcp_url().await;
        let cfg = McpServerConfig {
            url: Some(url),
            ..Default::default()
        };
        let client = McpClient::connect(&cfg).await.expect("connect");

        let tools = client.list_tools().await.expect("tools/list");
        assert!(
            tools.iter().any(|t| t.name == "memory_search"),
            "expected memory_search, got {:?}",
            tools.iter().map(|t| &t.name).collect::<Vec<_>>()
        );

        // Every discovered tool must carry a schema the model can fill in.
        for t in &tools {
            assert_eq!(
                t.input_schema.get("type").and_then(Value::as_str),
                Some("object"),
                "{}",
                t.name
            );
        }

        // A list, not a semantic search: a search loads the embedding model.
        let out = client
            .call_tool("memory_search", json!({ "op": "list", "limit": 1 }))
            .await
            .expect("tools/call");
        assert!(!out.starts_with("error:"), "{out}");
        // Rows come back as JSON inside a text block — the protocol's double
        // encoding, which a structured caller parses once more.
        serde_json::from_str::<Value>(&out).expect("rows should be JSON");
    }
}

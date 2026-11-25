//! Every connected server, and the tools they offer, for the life of the
//! daemon.
//!
//! Connection is async and happens once at startup; the tool list is read
//! synchronously on every turn (the advertised-schema path is not async).
//! Hence a snapshot behind a lock rather than dialling a server to answer
//! "what tools exist".
//!
//! Discovery is **best effort**. A server that is slow, dead, or misconfigured
//! costs its own tools and a line in the log — never a turn. That is not a
//! nicety: a user's MCP server is a third-party process we do not control, and
//! one of them hanging must not be able to own the agent.

use super::client::McpClient;
use super::config::McpServerConfig;
use crate::util::RwLockExt;
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, RwLock};
use std::time::Duration;
use tracing::{info, warn};

/// How long one server gets to connect and list its tools before we move on
/// without it. Startup is not the place to wait on someone else's process.
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(15);

/// The separator the ecosystem uses (`mcp__<server>__<tool>`). Kept rather
/// than invented: it is what models have seen from every other host, and a
/// familiar shape is one less thing for them to get wrong.
const PREFIX: &str = "mcp__";

/// A tool as the model sees it — server-qualified — plus what it takes to
/// call it.
#[derive(Debug, Clone)]
pub struct AdvertisedTool {
    /// `mcp__<server>__<tool>`.
    pub qualified: String,
    /// The server's own name for it, which is what goes back on the wire.
    pub original: String,
    pub server: String,
    pub description: String,
    pub input_schema: Value,
}

/// What became of one configured server. Kept whether or not it connected:
/// a server the engine tried and failed to reach must be visible with its
/// reason, not simply absent from the UI. Absent reads as "not configured",
/// which is a different and misleading thing.
#[derive(Debug, Clone)]
pub struct ServerStatus {
    pub name: String,
    /// `stdio: npx …` or the URL — what the user needs to recognise it.
    pub target: String,
    pub connected: bool,
    /// Why it isn't connected, when it isn't.
    pub error: Option<String>,
    pub tools: Vec<String>,
    pub enabled: bool,
    /// Whether this server's tools go through the permission gate. Shown
    /// because a tab that adds a server without saying what it may do is a
    /// phantom: real capability, invisible.
    pub gated: bool,
}

#[derive(Default)]
struct State {
    clients: BTreeMap<String, Arc<McpClient>>,
    tools: Vec<AdvertisedTool>,
    servers: Vec<ServerStatus>,
}

pub struct McpRegistry {
    state: RwLock<State>,
}

static REGISTRY: LazyLock<McpRegistry> = LazyLock::new(|| McpRegistry {
    state: RwLock::new(State::default()),
});

pub fn registry() -> &'static McpRegistry {
    &REGISTRY
}

impl McpRegistry {
    /// A registry of its own, connected to nothing — for a test that must
    /// not touch the process's one.
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self {
            state: RwLock::new(State::default()),
        }
    }
}

pub fn qualify(server: &str, tool: &str) -> String {
    format!("{PREFIX}{server}__{tool}")
}

/// Is this a name we serve? Cheap enough to ask before a map lookup.
pub fn is_mcp_tool(name: &str) -> bool {
    name.starts_with(PREFIX)
}

impl McpRegistry {
    /// Connect every enabled server and record what it offers.
    ///
    /// Replaces the previous set wholesale, so this doubles as reload.
    /// Servers are dialled concurrently: a slow one shouldn't delay the rest.
    ///
    /// Every server here is the user's own, from `[mcp_servers]` — there is
    /// no second scope to merge in. See `config.rs` for why a repo's
    /// `.mcp.json` is not read.
    pub async fn connect_all(&self, configs: &BTreeMap<String, McpServerConfig>) {
        let mut set = tokio::task::JoinSet::new();
        for (name, cfg) in configs.iter().map(|(n, c)| (n.clone(), c.clone())) {
            set.spawn(async move {
                let target = describe(&cfg);
                let gated = cfg.gated;
                if !cfg.enabled {
                    return Attempt {
                        name,
                        target,
                        enabled: false,
                        gated,
                        found: None,
                        error: None,
                    };
                }
                let outcome = tokio::time::timeout(DISCOVERY_TIMEOUT, discover(&name, &cfg)).await;
                let (found, error) = match outcome {
                    Ok(Ok(f)) => (Some(f), None),
                    Ok(Err(e)) => (None, Some(format!("{e:#}"))),
                    Err(_) => (None, Some(format!("no answer in {DISCOVERY_TIMEOUT:?}"))),
                };
                Attempt {
                    name,
                    target,
                    enabled: true,
                    gated,
                    found,
                    error,
                }
            });
        }

        let mut clients = BTreeMap::new();
        let mut tools = Vec::new();
        let mut servers = Vec::new();
        while let Some(joined) = set.join_next().await {
            let Ok(attempt) = joined else { continue };
            let Attempt {
                name,
                target,
                enabled,
                gated,
                found,
                error,
            } = attempt;
            match found {
                Some(found) => {
                    info!("MCP `{name}` connected — {} tool(s)", found.tools.len());
                    servers.push(ServerStatus {
                        name: name.clone(),
                        target,
                        connected: true,
                        error: None,
                        tools: found.tools.iter().map(|t| t.qualified.clone()).collect(),
                        enabled,
                        gated,
                    });
                    tools.extend(found.tools);
                    clients.insert(name, found.client);
                }
                None => {
                    if let Some(e) = &error {
                        warn!("MCP `{name}` unavailable: {e}");
                    }
                    servers.push(ServerStatus {
                        name,
                        target,
                        connected: false,
                        error,
                        tools: Vec::new(),
                        enabled,
                        gated,
                    });
                }
            }
        }
        servers.sort_by(|a, b| a.name.cmp(&b.name));

        *self.state.write_ok() = State {
            clients,
            tools,
            servers,
        };
    }

    /// Every configured server and what became of it — what the Settings tab
    /// renders, including the ones that failed.
    pub fn status(&self) -> Vec<ServerStatus> {
        self.state.read_ok().servers.clone()
    }

    /// Everything currently on offer, for the advertised tool list.
    pub fn advertised(&self) -> Vec<AdvertisedTool> {
        self.state.read_ok().tools.clone()
    }

    /// One tool as discovery found it — its owning server and the schema that
    /// server advertised for it.
    ///
    /// For a caller that fills in arguments the model didn't: the schema is
    /// the server's own statement of what it accepts, so a field it never
    /// declared is never invented for it.
    pub fn advertised_tool(&self, qualified: &str) -> Option<AdvertisedTool> {
        self.state
            .read_ok()
            .tools
            .iter()
            .find(|t| t.qualified == qualified)
            .cloned()
    }

    /// Which server owns a qualified name, and what it calls the tool.
    fn route(&self, qualified: &str) -> Option<(Arc<McpClient>, String)> {
        let state = self.state.read_ok();
        let tool = state.tools.iter().find(|t| t.qualified == qualified)?;
        let client = state.clients.get(&tool.server)?.clone();
        Some((client, tool.original.clone()))
    }

    /// Call a tool by its qualified name.
    pub async fn call(&self, qualified: &str, args: Value) -> Result<String> {
        let (client, original) = self
            .route(qualified)
            .ok_or_else(|| anyhow!("no MCP server offers `{qualified}`"))?;
        client.call_tool(&original, args).await
    }

    /// Does this qualified tool go through the permission gate?
    ///
    /// Answered from the owning server's `gated` field, so the gate reads the
    /// same declaration the config and the Settings tab show. An unknown name
    /// is gated: a tool we cannot attribute is not one to wave through.
    pub fn is_gated(&self, qualified: &str) -> bool {
        let state = self.state.read_ok();
        let Some(tool) = state.tools.iter().find(|t| t.qualified == qualified) else {
            return true;
        };
        state
            .servers
            .iter()
            .find(|s| s.name == tool.server)
            .map(|s| s.gated)
            .unwrap_or(true)
    }

    /// Doctrine each connected server shipped in `initialize`, as
    /// (server, instructions). This is how a server states its own rules
    /// instead of the host hand-copying them.
    pub fn instructions(&self) -> Vec<(String, String)> {
        let state = self.state.read_ok();
        state
            .clients
            .iter()
            .filter_map(|(name, c)| c.instructions().map(|i| (name.clone(), i.to_string())))
            .collect()
    }
}

struct Attempt {
    name: String,
    target: String,
    enabled: bool,
    gated: bool,
    found: Option<Discovered>,
    error: Option<String>,
}

/// How a server is addressed, for the user to recognise it by.
fn describe(cfg: &McpServerConfig) -> String {
    match cfg.transport() {
        Ok(super::config::Transport::Stdio { command, args, .. }) => {
            if args.is_empty() {
                format!("stdio: {command}")
            } else {
                format!("stdio: {command} {}", args.join(" "))
            }
        }
        Ok(super::config::Transport::Http { url, .. }) => url,
        Err(why) => format!("misconfigured — {why}"),
    }
}

struct Discovered {
    client: Arc<McpClient>,
    tools: Vec<AdvertisedTool>,
}

async fn discover(name: &str, cfg: &McpServerConfig) -> Result<Discovered> {
    let client = Arc::new(McpClient::connect(cfg).await?);
    let tools = client
        .list_tools()
        .await?
        .into_iter()
        .map(|t| AdvertisedTool {
            qualified: qualify(name, &t.name),
            original: t.name,
            server: name.to_string(),
            description: t.description,
            input_schema: t.input_schema,
        })
        .collect();
    Ok(Discovered { client, tools })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn qualified_names_keep_two_servers_apart() {
        assert_eq!(qualify("github", "search"), "mcp__github__search");
        assert_ne!(qualify("github", "search"), qualify("sentry", "search"));
        assert!(is_mcp_tool("mcp__github__search"));
        assert!(!is_mcp_tool("Read"));
        assert!(!is_mcp_tool("AskUser"));
    }

    /// An unconnected registry answers rather than panics — the state every
    /// test process and every fresh daemon starts in.
    #[tokio::test]
    async fn an_empty_registry_offers_nothing_and_routes_nowhere() {
        let empty = McpRegistry {
            state: RwLock::new(State::default()),
        };
        assert!(empty.advertised().is_empty());
        assert!(empty.instructions().is_empty());
        let err = empty
            .call("mcp__nope__x", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no MCP server offers"), "{err}");
    }

    /// A configured-but-unreachable server must not stall or poison the set:
    /// discovery is best effort, and the daemon carries on without it.
    #[tokio::test]
    async fn an_unreachable_server_costs_only_its_own_tools() {
        let reg = McpRegistry {
            state: RwLock::new(State::default()),
        };
        let mut cfgs = BTreeMap::new();
        cfgs.insert(
            "dead".to_string(),
            McpServerConfig {
                // Nothing listens here; connect fails fast rather than hanging.
                url: Some("http://127.0.0.1:9/mcp".into()),
                ..Default::default()
            },
        );
        cfgs.insert(
            "parked".to_string(),
            McpServerConfig {
                url: Some("http://127.0.0.1:9/mcp".into()),
                enabled: false,
                ..Default::default()
            },
        );
        reg.connect_all(&cfgs).await;
        assert!(reg.advertised().is_empty());
    }

    /// The whole chain in one process: config -> connect -> discover ->
    /// advertise -> call. Populates the REAL global registry, which is what
    /// the advertised-tool path reads, so this proves the wiring and not
    /// just the pieces. The server is the hosted memory on a temp store.
    #[tokio::test]
    async fn a_configured_server_reaches_the_model_s_tool_list() {
        let _registry = crate::mcp_client::test_memory::global_registry().await;
        let (url, _store) = crate::mcp_client::test_memory::mcp_url().await;

        let mut cfgs = BTreeMap::new();
        cfgs.insert(
            "ling-mem".to_string(),
            McpServerConfig {
                url: Some(url),
                ..Default::default()
            },
        );
        registry().connect_all(&cfgs).await;

        // Discovered, and server-qualified so two servers could both offer it.
        let want = qualify("ling-mem", "memory_search");
        assert!(
            registry().advertised().iter().any(|t| t.qualified == want),
            "advertised: {:?}",
            registry()
                .advertised()
                .iter()
                .map(|t| &t.qualified)
                .collect::<Vec<_>>()
        );

        // Reaches the list the model is actually shown.
        let defs = crate::engine::tools::json_schema::oai_tool_definitions(None);
        let names: Vec<&str> = defs
            .iter()
            .filter_map(|d| d.get("function")?.get("name")?.as_str())
            .collect();
        assert!(
            names.contains(&want.as_str()),
            "not advertised to the model"
        );
        // …alongside the built-ins, not instead of them.
        assert!(names.contains(&"Read"));

        // And it dispatches by that qualified name.
        let out = registry()
            .call(
                &qualify("ling-mem", "memory_search"),
                serde_json::json!({"op": "list", "limit": 1}),
            )
            .await
            .expect("dispatch");
        serde_json::from_str::<serde_json::Value>(&out).expect("rows should be JSON");

        // The server's own doctrine came across on initialize — the hook
        // phase 2 needs to stop hand-copying the memory protocol.
        assert!(
            registry()
                .instructions()
                .iter()
                .any(|(s, i)| s == "ling-mem" && !i.is_empty()),
            "expected ling-mem to ship instructions"
        );

        // Leave the global registry as we found it for other tests.
        registry().connect_all(&BTreeMap::new()).await;
    }

    /// A memory store of the test's own, served on a free loopback port the
    /// way the engine serves it (memory's router, MCP at `/mcp`) — never the
    /// person's. Returns the folder (kept alive by the caller) and the MCP URL.
    async fn scratch_memory() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let state = ling_mem::hosted::open_quiet(dir.path(), base.clone())
            .await
            .unwrap();
        let app = ling_mem::hosted::router(state)
            .into_make_service_with_connect_info::<std::net::SocketAddr>();
        tokio::spawn(async move { axum::serve(listener, app).await });
        (dir, format!("{base}/mcp"))
    }

    /// Auto-recall's exact call shape, including `min_score` — which is
    /// deliberately NOT in `memory_search`'s advertised schema, because a
    /// model guessing a relevance threshold narrows recall to nothing.
    ///
    /// That makes it the one argument whose passthrough isn't covered by the
    /// schema, so it is the one worth a test: if memory's MCP layer ever
    /// dropped unadvertised fields, recall would silently lose its floor and
    /// inject weak rows. It embeds, so it needs the embedding model.
    #[tokio::test]
    #[ignore = "needs the embedding model in ~/.cache/huggingface"]
    async fn auto_recalls_min_score_survives_the_mcp_passthrough() {
        let (_dir, url) = scratch_memory().await;
        let reg = McpRegistry {
            state: RwLock::new(State::default()),
        };
        let mut cfgs = BTreeMap::new();
        cfgs.insert(
            "memory".to_string(),
            McpServerConfig {
                url: Some(url.clone()),
                ..Default::default()
            },
        );
        reg.connect_all(&cfgs).await;
        let facts: Vec<Value> = [
            "Linggen memory keeps who the user is across sessions.",
            "Linggen's memory store holds core and long-term rows.",
            "The nightly dream promotes Linggen memory from staging.",
            "Linggen memory is shared by every coding host on the machine.",
        ]
        .iter()
        .map(|c| serde_json::json!({"content": c, "type": "fact", "tier": "semantic"}))
        .collect();
        let added = reqwest::Client::new()
            .post(url.replace("/mcp", "/api/memory/add_batch"))
            .json(&serde_json::json!({ "facts": facts }))
            .send()
            .await
            .expect("seed the scratch store");
        assert!(added.status().is_success(), "seeding: {}", added.status());
        let tool = qualify("memory", "memory_search");

        let rows = |v: &str| -> usize {
            serde_json::from_str::<Value>(v)
                .ok()
                .and_then(|j| j.as_array().map(|a| a.len()))
                .unwrap_or(0)
        };

        let loose = reg
            .call(
                &tool,
                serde_json::json!({"query": "linggen memory", "limit": 10, "min_score": 0.0}),
            )
            .await
            .expect("dispatch");
        // A threshold almost nothing clears. Not asserted to be *zero*: the
        // hybrid score is cosine plus a keyword boost, so a row that matches
        // the query closely can saturate near 1.0. What must hold is that the
        // floor bit at all — if the field were dropped on the way through,
        // this would come back identical to the loose call.
        let strict = reg
            .call(
                &tool,
                serde_json::json!({"query": "linggen memory", "limit": 10, "min_score": 0.999}),
            )
            .await
            .expect("dispatch");

        assert!(
            rows(&loose) > 1,
            "need a few rows for this to mean anything: {loose}"
        );
        assert!(
            rows(&strict) < rows(&loose),
            "the floor did nothing — loose={} strict={}",
            rows(&loose),
            rows(&strict)
        );
    }

    /// The gate reads the owning server's declaration. Two servers, one
    /// ungated, so a pass can't come from a global default.
    #[tokio::test]
    async fn the_gate_follows_the_servers_own_declaration() {
        let (_dir, url) = scratch_memory().await;
        let reg = McpRegistry {
            state: RwLock::new(State::default()),
        };
        let mut cfgs = BTreeMap::new();
        cfgs.insert(
            "trusted".to_string(),
            McpServerConfig {
                url: Some(url.clone()),
                gated: false,
                ..Default::default()
            },
        );
        cfgs.insert(
            "guarded".to_string(),
            McpServerConfig {
                url: Some(url),
                ..Default::default()
            },
        );
        reg.connect_all(&cfgs).await;

        assert!(!reg.is_gated(&qualify("trusted", "memory_search")));
        assert!(reg.is_gated(&qualify("guarded", "memory_search")));
        // Same tool name, opposite answers — so the lookup is by server, not
        // by anything about the tool itself.

        // A name we can't attribute is gated: fail closed.
        assert!(reg.is_gated("mcp__ghost__whatever"));
        assert!(reg.is_gated("Read"));
    }
}

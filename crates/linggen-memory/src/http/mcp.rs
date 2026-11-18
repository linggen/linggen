//! MCP (Model Context Protocol) Streamable-HTTP endpoint.
//!
//! Exposes the daemon's memory operations as MCP tools so Claude Code,
//! Codex, Cursor, and any MCP client can register `ling-mem` as a
//! provider via `.mcp.json` without spawning a stdio subprocess.
//!
//! Single endpoint at `POST /mcp` accepting JSON-RPC 2.0. Methods:
//! - `initialize` — protocol handshake, returns server capabilities.
//! - `tools/list` — enumerate tools with their input schemas.
//! - `tools/call` — execute a tool. The verb is answered as its route
//!   `/api/memory/<verb>` answers it (`http::verb::answer`, through
//!   `Memory::call`, no request made), so MCP calls share the routes'
//!   dispatch and refusals.
//!
//! Notifications (no `id`, no response expected) return HTTP 204.
//!
//! Tool schemas are deliberately narrow — only fields each verb actually
//! consumes — to defend against models that over-fill optional fields
//! with hallucinated defaults (see the past_ttl-sweep guard below and
//! the gpt-5.5 case in DESIGN.md).

use super::state::SharedState;
use crate::catalog::memory::{self, catalog};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "memory";

/// Always-on primer injected into the client's system prompt at session
/// start. The MCP spec's `instructions` field is the daemon's way to teach
/// every connected host (Claude Code, Codex, Cursor, …) the ling-mem
/// doctrine without per-host CLAUDE.md edits.
///
/// HARD CAP: 2048 characters. Claude Code cuts server instructions there and
/// drops the rest silently — the old 9356-character protocol reached CC
/// sessions only through its tier definitions, so the routing rules (role →
/// core, search first, the merge law) never arrived. Only rules that decide
/// a save live here; the long form, with examples and the cleanup taxonomy,
/// is the memory skill. `instructions_fit_claude_codes_cap` holds the line.
const INSTRUCTIONS: &str = r#"This is the user's memory, shared by Claude Code, Codex, OpenClaw and Linggen. Save what makes a future session predict the user better — the person, not the task.

# Tiers
- core — who they are, always loaded: name, role/job, location, timezone, languages, family/pets. One short fact per row.
- semantic — durable, recalled on demand: goals, preferences ("always X / never Y" → type=preference, from=user; never core), decisions with their why, gotchas.
- episodic — the default capture (omit tier): events, milestones, run learnings. No search first; the nightly dream promotes what lasts.

# Saving
- Before a core/semantic add, memory_search the subject. Same subject or claim in other words = conflict; when unsure, treat it as one.
- Your own rows (from=derived): merge into one current-truth row via memory_add + replace_ids, never add then delete.
- The user's rows (from=user): ask first, showing each row's full text and date; then replace_ids + user_directed:true. Never from your own inference.
- What the user said is from=user. A replacement keeps the loser's tier and scope. A new status (shipped/fixed/dropped) replaces the old status row.
- "remember / forget / update X", in any language: search, act, user_directed:true.
- Anchor relative time to dates ("last month" → "2026-08"). occurred_at dates days and TTL; it never ranks.
- A row's scope is the directory it is about (default: the host's cwd). Pass scope (one of "Memory scopes here") when it isn't where you stand; global:true when it is about the person.
- A standing rule the user states ("always…", "以后都…") gets indexed:true and a summary (one line ≤ 80 chars) — the index loads it at session start under its scope.
- Never save secrets or file bodies you can re-read. Project internals stay episodic.

# Recall
Search when the question may touch past preferences, decisions or gotchas. Show each fact you use: "From memory (3 months ago): …". Never pass type, from or outcome to memory_search unless asked.

Full rules and examples: the memory skill."#;

pub fn router() -> Router<SharedState> {
    Router::new().route("/mcp", post(handler))
}

// ── JSON-RPC dispatch ───────────────────────────────────────────────────────

/// Single entrypoint. Parses the JSON-RPC envelope, routes by `method`,
/// and returns either a JSON-RPC response (200) or 204 for notifications.
async fn handler(State(state): State<SharedState>, Json(req): Json<Value>) -> Response {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(Value::Null);

    // Notifications: no `id` → no response, just 204. The most common is
    // `notifications/initialized` from the client after the handshake.
    if id.is_none() {
        return StatusCode::NO_CONTENT.into_response();
    }

    let result = match method {
        "initialize" => Ok(handle_initialize()),
        "tools/list" => Ok(handle_tools_list()),
        "tools/call" => handle_tools_call(&state, params).await,
        other => Err(rpc_error(-32601, format!("method not found: {other}"))),
    };

    let envelope = match result {
        Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
        Err(err) => json!({ "jsonrpc": "2.0", "id": id, "error": err }),
    };
    Json(envelope).into_response()
}

fn rpc_error(code: i32, message: impl Into<String>) -> Value {
    json!({ "code": code, "message": message.into() })
}

// ── initialize ──────────────────────────────────────────────────────────────

fn handle_initialize() -> Value {
    json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": {
            "name": SERVER_NAME,
            "version": env!("CARGO_PKG_VERSION"),
        },
        "instructions": INSTRUCTIONS,
    })
}

// ── tools/list ──────────────────────────────────────────────────────────────

/// The names of the tools this server lists, in order.
pub fn tool_names() -> Vec<String> {
    catalog()
        .legacy()
        .filter_map(|b| b.legacy.map(str::to_string))
        .collect()
}

fn handle_tools_list() -> Value {
    json!({ "tools": tool_defs() })
}

/// The tools this server publishes: memory's branches, each as the tool it
/// has been since 1.x (`catalog::memory`). Narrow schemas — only fields the
/// corresponding `/api/memory/<verb>` handler actually consumes for that
/// operation, so models can't over-fill speculative ones.
fn tool_defs() -> Vec<Value> {
    catalog()
        .legacy()
        .filter_map(|b| b.legacy_def())
        .collect()
}

// ── tools/call ──────────────────────────────────────────────────────────────

/// A legacy call: its branch found by the tool's name, its arguments taken
/// as they always were (repaired to the schema's types, never refused for
/// a field the schema does not show), then run as the branch runs.
async fn handle_tools_call(state: &SharedState, params: Value) -> Result<Value, Value> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| rpc_error(-32602, "tools/call requires `name`"))?
        .to_string();
    let mut args = params.get("arguments").cloned().unwrap_or(json!({}));

    let branch = catalog()
        .legacy_branch(&name)
        .ok_or_else(|| rpc_error(-32602, format!("unknown tool: {name}")))?;

    memory::coerce_legacy(branch, &mut args);
    match memory::run(state, branch, args).await {
        Ok(data) => Ok(memory::text_content(&data)),
        Err(err) => Err(rpc_error(-32603, err.message)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::memory::read_answer;
    use crate::http::envelope::ApiError;

    // The legacy names these tests have always called, over the catalogue.
    fn timed_out(verb: &str) -> ApiError {
        let wait = std::time::Duration::from_millis(memory::CALL_TIMEOUT_MS);
        memory::timed_out(verb, wait)
    }
    fn tool_name_to_verb(name: &str) -> Option<&'static str> {
        catalog().legacy_branch(name).map(|b| b.dispatch)
    }
    fn coerce_to_schema(tool: &str, args: &mut Value) {
        if let Some(branch) = catalog().legacy_branch(tool) {
            memory::coerce_legacy(branch, args);
        }
    }
    fn apply_dispatch_fixes(args: &mut Value) {
        memory::prepare(args);
    }
    fn apply_response_fixes(verb: &str, value: &mut Value) {
        memory::finish(verb, value);
    }

    /// A verb that outlives the wait: the caller is told its write may
    /// have landed.
    #[test]
    fn a_timed_out_call_says_the_write_may_have_landed() {
        let message = timed_out("add").message;
        assert!(message.starts_with("add timed out after 25s"), "{message}");
        assert!(message.contains("may still have landed"), "{message}");
    }

    /// The 2026-10-04 Claude Code call: string-typed booleans, numbers and
    /// a lone id must reach the daemon as the schema's types.
    #[test]
    fn string_typed_args_take_the_schemas_types() {
        let mut args = json!({
            "content": "x",
            "scope": "~/workspace/linggen/linggen",
            "summary": "s",
            "indexed": "false",
            "global": "TRUE",
            "replace_ids": "abc123",
            "user_directed": "maybe"
        });
        coerce_to_schema("memory_add", &mut args);
        assert_eq!(args["indexed"], json!(false));
        assert_eq!(args["global"], json!(true));
        assert_eq!(args["replace_ids"], json!(["abc123"]));
        assert_eq!(args["scope"], json!("~/workspace/linggen/linggen"));
        assert_eq!(args["summary"], json!("s"));
        // Not a boolean in any spelling: left for the daemon to name.
        assert_eq!(args["user_directed"], json!("maybe"));

        let mut search = json!({"query": "q", "limit": "5"});
        coerce_to_schema("memory_search", &mut search);
        assert_eq!(search["limit"], json!(5));
        let mut list = json!({"indexed": "true", "offset": "10"});
        coerce_to_schema("memory_list", &mut list);
        assert_eq!(list["indexed"], json!(true));
        assert_eq!(list["offset"], json!(10));

        let mut ids = json!({"replace_ids": "[\"a\",\"b\"]"});
        coerce_to_schema("memory_add", &mut ids);
        assert_eq!(ids["replace_ids"], json!(["a", "b"]));
    }

    /// Params the REST DTO takes but the schema does not show (2026-10-05:
    /// `skip_dedup: "true"` was a 422): a boolean spelled as a string is
    /// a boolean; any other string stays as sent.
    #[test]
    fn unadvertised_string_booleans_become_booleans() {
        let mut args = json!({
            "content": "true",
            "skip_dedup": "true",
            "force": "FALSE",
            "host": "codex",
            "source_session": "abc"
        });
        coerce_to_schema("memory_add", &mut args);
        assert_eq!(args["skip_dedup"], json!(true));
        assert_eq!(args["force"], json!(false));
        // Advertised strings keep their type, whatever they spell.
        assert_eq!(args["content"], json!("true"));
        assert_eq!(args["host"], json!("codex"));
        assert_eq!(args["source_session"], json!("abc"));
        let mut sent = json!({"content": "x", "skip_dedup": "true"});
        coerce_to_schema("memory_add", &mut sent);
        let req: crate::http::memory::AddRequest = serde_json::from_value(sent).unwrap();
        assert!(req.skip_dedup);
    }

    /// A plain-text refusal from the route reaches the caller verbatim.
    #[tokio::test]
    async fn a_plain_text_refusal_is_reported_as_said() {
        let refusal = (
            StatusCode::UNPROCESSABLE_ENTITY,
            "Failed to deserialize the JSON body: indexed: invalid type",
        )
            .into_response();
        let err = read_answer("add", refusal).await.unwrap_err();
        assert!(err.message.contains("HTTP 422"), "{}", err.message);
        assert!(
            err.message.contains("indexed: invalid type"),
            "{}",
            err.message
        );
    }

    /// `until: ""` used to reach the RFC-3339 parser and crash it; empty
    /// arrays silently narrowed a query to nothing.
    #[test]
    fn soft_empty_fields_never_reach_the_daemon() {
        let mut args = json!({
            "query": "real",
            "until": "",
            "types": [],
            "tier": null,
            "limit": 5
        });
        apply_dispatch_fixes(&mut args);
        // Key order is serde_json's: sorted, or as inserted where a crate in
        // the build turns on `preserve_order` (the engine does; lancedb 0.41
        // does too). Which keys are left is what counts.
        let mut keys: Vec<_> = args.as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(keys, ["limit", "query"]);
    }

    /// The episodic store is a separate table, so the tier is a routing
    /// decision on the wire, not a filter value.
    #[test]
    fn episodic_tier_becomes_a_routing_flag() {
        let mut args = json!({"content": "x", "tier": "episodic"});
        apply_dispatch_fixes(&mut args);
        assert_eq!(args["episodic"], json!(true));
        assert!(args.get("tier").is_none());

        // Any other tier stays a filter within the semantic store.
        let mut semantic = json!({"content": "x", "tier": "core"});
        apply_dispatch_fixes(&mut semantic);
        assert_eq!(semantic["tier"], json!("core"));
        assert!(semantic.get("episodic").is_none());
    }

    /// A sweep wants every past-TTL row. gpt-5.5 was observed filling these
    /// with hallucinated defaults, narrowing it to zero.
    #[test]
    fn a_ttl_sweep_is_not_narrowed_by_invented_filters() {
        let mut args = json!({
            "past_ttl": true,
            "type": "fact",
            "from": "user",
            "outcome": "positive",
            "scope_root": "/keep/me"
        });
        apply_dispatch_fixes(&mut args);
        assert!(args.get("type").is_none());
        assert!(args.get("from").is_none());
        assert!(args.get("outcome").is_none());
        // A caller legitimately scoping by path keeps it.
        assert_eq!(args["scope_root"], json!("/keep/me"));

        // Not gated on the verb: the same shape on the wrong one is stripped.
        let mut wrong_verb = json!({"past_ttl": true, "type": "fact"});
        apply_dispatch_fixes(&mut wrong_verb);
        assert!(wrong_verb.get("type").is_none());
    }

    /// Three dream runs aborted claiming "store inconsistency" over a bare
    /// `removed:false`. The row is gone either way.
    #[test]
    fn deleting_an_absent_row_reads_as_success() {
        let mut resp = json!({"id": "abc", "removed": false});
        apply_response_fixes("delete", &mut resp);
        assert_eq!(resp["already_gone"], json!(true));
        assert!(resp["note"].as_str().unwrap().contains("treat as success"));

        // A real delete is left alone…
        let mut real = json!({"id": "abc", "removed": true});
        apply_response_fixes("delete", &mut real);
        assert!(real.get("already_gone").is_none());

        // …and so is every other verb's response.
        let mut search = json!({"removed": false});
        apply_response_fixes("search", &mut search);
        assert!(search.get("already_gone").is_none());
    }

    /// An advertised tool nobody can route is a phantom, and a routable verb
    /// nobody advertises is a capability the model cannot reach — which is
    /// exactly how `issue_add` went missing while every other verb had a
    /// tool. The two lists are one surface; test them as one.
    #[test]
    fn every_advertised_tool_routes_and_every_route_is_advertised() {
        let advertised: Vec<String> = tool_defs()
            .iter()
            .map(|t| t["name"].as_str().expect("a tool has a name").to_string())
            .collect();

        for name in &advertised {
            assert!(
                tool_name_to_verb(name).is_some(),
                "advertised tool `{name}` has no verb to route to"
            );
        }

        // Every verb the map knows must be advertised. Kept as the map's own
        // inverse rather than a hand-written list, so adding a route without
        // a schema fails here.
        for verb in [
            "search",
            "list",
            "session_start",
            "get",
            "add",
            "update",
            "delete",
            "days",
            "remember_day",
            "harvest_day",
            "sweep",
            "dream_lease",
            "role",
            "chains",
            "issues",
            "issue_add",
            "issue_resolve",
        ] {
            let advertised_for_verb = advertised
                .iter()
                .any(|name| tool_name_to_verb(name) == Some(verb));
            assert!(
                advertised_for_verb,
                "verb `{verb}` is routable but not advertised"
            );
        }
    }

    /// The audit queues what it cannot solve; without this tool the whole
    /// review queue is write-only from a model's point of view.
    #[test]
    fn the_audit_can_queue_a_review_item() {
        assert_eq!(tool_name_to_verb("memory_issue_add"), Some("issue_add"));
        let def = tool_defs()
            .into_iter()
            .find(|t| t["name"] == json!("memory_issue_add"))
            .expect("memory_issue_add is advertised");
        // `kind` + `note` are what /api/memory/issue_add rejects a call for
        // missing, so they are the schema's required pair.
        assert_eq!(def["inputSchema"]["required"], json!(["kind", "note"]));
    }

    /// Claude Code cuts server instructions at 2048 characters and drops the
    /// rest without a word. Past the cap, the rules at the end are the ones
    /// a CC session never sees.
    #[test]
    fn instructions_fit_claude_codes_cap() {
        let n = INSTRUCTIONS.chars().count();
        assert!(
            n <= 2048,
            "instructions are {n} characters; Claude Code keeps 2048"
        );
    }

    /// `from` was missing from the add schema, so every row a model wrote
    /// defaulted to derived — the user's own words included.
    #[test]
    fn add_takes_from_scope_summary_and_indexed() {
        let def = tool_defs()
            .into_iter()
            .find(|t| t["name"] == json!("memory_add"))
            .unwrap();
        let props = def["inputSchema"]["properties"].as_object().unwrap();
        for f in [
            "from",
            "scope",
            "summary",
            "indexed",
            "cwd",
            "root",
            "source_session",
            "host",
        ] {
            assert!(props.contains_key(f), "memory_add lacks {f}");
        }
        assert!(!props.contains_key("contexts"));
        let all = serde_json::to_string(&tool_defs()).unwrap();
        assert!(
            !all.contains("\"contexts\""),
            "a tool still advertises contexts"
        );
        assert!(!all.contains("\"tags\""), "a tool still advertises tags");
        assert!(!all.contains("\"hook\""), "a tool still advertises hook");
    }

    /// JSON with every object's keys sorted, so a fingerprint does not depend
    /// on whether a build preserves insertion order.
    fn canonical(v: &Value) -> String {
        match v {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                let fields: Vec<String> = keys
                    .iter()
                    .map(|k| format!("{}:{}", Value::String((*k).clone()), canonical(&map[*k])))
                    .collect();
                format!("{{{}}}", fields.join(","))
            }
            Value::Array(items) => {
                let items: Vec<String> = items.iter().map(canonical).collect();
                format!("[{}]", items.join(","))
            }
            other => other.to_string(),
        }
    }

    fn fingerprint(v: &Value) -> String {
        let mut h = crate::hash::Sha256::new();
        h.update(canonical(v).as_bytes());
        h.hex()[..16].to_string()
    }

    /// The seventeen tools this server publishes, each with its whole
    /// definition (name, description, input schema) fingerprinted, and the
    /// server text. Building the one-door catalogue must not change any of
    /// them by accident: a change here is a change every connected host sees.
    #[test]
    fn the_published_surface_is_locked() {
        let actual: Vec<(String, String)> = tool_defs()
            .iter()
            .map(|t| (t["name"].as_str().unwrap().to_string(), fingerprint(t)))
            .collect();
        let expected: &[(&str, &str)] = &[
            ("memory_search", "72943d6bae379810"),
            ("memory_list", "df0c71a498197be7"),
            ("memory_session_start", "6308e35d63ff8472"),
            ("memory_get", "3ac7e4acb602b7bf"),
            ("memory_add", "f4589342f5c221a1"),
            ("memory_delete", "1f7a19393cacf49e"),
            ("memory_update", "b4c034fa182c3d7e"),
            ("memory_harvest_day", "74d4782e4ac36645"),
            ("memory_days", "3970ca5e8ae8d6bb"),
            ("memory_remember_day", "98fb0e4e78cb3a7f"),
            ("memory_dream_lease", "bc03543d4aeec717"),
            ("memory_role", "e39c3e46fc9889f0"),
            ("memory_sweep", "ad9e4a884524b660"),
            ("memory_issues", "817c0942710abaf5"),
            ("memory_issue_add", "8184b55c04553ab4"),
            ("memory_issue_resolve", "0f967175747b4007"),
            ("memory_chains", "f558e7bae94b4ab0"),
        ];
        let shown: Vec<String> = actual
            .iter()
            .map(|(n, f)| format!("(\"{n}\", \"{f}\"),"))
            .collect();
        let want: Vec<(String, String)> = expected
            .iter()
            .map(|(n, f)| (n.to_string(), f.to_string()))
            .collect();
        assert_eq!(actual, want, "actual:\n{}", shown.join("\n"));
        let text = fingerprint(&Value::String(INSTRUCTIONS.to_string()));
        assert_eq!(text, "7fb7025cd50abf5f", "instructions changed: {text}");
    }

    /// The catalogue's entry is the only place a tool's name decides
    /// anything: this server's dispatch keeps no name → verb list of its own.
    #[test]
    fn nothing_outside_the_catalogue_matches_on_a_tool_name() {
        let code = include_str!("mcp.rs").split("#[cfg(test)]").next().unwrap();
        for name in tool_names() {
            let arm = format!("\"{name}\" =>");
            assert!(!code.contains(&arm), "http/mcp.rs matches on {name}");
            let def = format!("\"name\": \"{name}\"");
            assert!(!code.contains(&def), "http/mcp.rs defines {name} itself");
        }
    }

    /// The routing rule whose loss started this: a role reaches core.
    #[test]
    fn instructions_route_a_role_to_core() {
        let core = INSTRUCTIONS
            .lines()
            .find(|l| l.starts_with("- core"))
            .expect("a core tier line");
        assert!(core.contains("role/job"), "{core}");
    }
}

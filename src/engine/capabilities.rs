//! Capability registry — canonical tool contracts implemented by
//! pluggable skills.
//!
//! A **capability** is a named set of tools the engine defines once and
//! any number of skills implement. Skills declare `provides: [<name>]`
//! and an `implements:` block mapping each tool to its HTTP endpoint on
//! the skill's daemon. The engine owns: tool names, argument schemas,
//! descriptions, and permission tiers. Skills own: the backend URL + the
//! wire path per tool.
//!
//! This matches the "canonical contract" rule in `doc/memory-spec.md` —
//! swapping providers preserves the model's tool surface identically.
//! Capability tools appear in the model's tool list (with schemas) only
//! when an active `provides: [<name>]` skill is installed and the
//! session's prompt profile opts into the capability (owner sessions do;
//! consumer/mission sessions don't).
//!
//! Adding a new capability = appending one more entry to `CAPABILITIES`.
//! Adding a new tool to an existing capability = appending one more
//! `CapabilityTool` + updating any skill's `implements:` block that
//! claims to cover the whole capability. The engine validates neither
//! shape today; future work: an engine-side check that each active
//! provider's `implements:` covers every declared tool in the capability.

use super::permission::PermissionMode;
use serde_json::{json, Value};
use std::sync::LazyLock;

/// One tool in a capability's canonical contract.
#[derive(Debug, Clone)]
pub struct CapabilityTool {
    pub name: String,
    pub description: String,
    pub tier: PermissionMode,
    /// JSON-Schema for the tool's arguments. Used directly for native
    /// tool calling; converted to the legacy "args: {k: 'type'}" shape
    /// for legacy JSON-action format.
    pub args_schema: Value,
}

/// A named capability — a set of tools the engine defines, a skill
/// implements.
#[derive(Debug, Clone)]
pub struct Capability {
    pub name: String,
    pub tools: Vec<CapabilityTool>,
}

/// All capabilities the engine knows about. Skills must use these exact
/// tool names; schemas here are the truth the model sees.
pub static CAPABILITIES: LazyLock<Vec<Capability>> = LazyLock::new(build_capabilities);

fn build_capabilities() -> Vec<Capability> {
    vec![memory_capability()]
}

// ── Memory capability ──────────────────────────────────────────────────

fn memory_capability() -> Capability {
    Capability {
        name: "memory".to_string(),
        tools: vec![
            CapabilityTool {
                name: "Memory_query".to_string(),
                description: "Read memory. Verb-dispatched: `get` (fetch one row by id), `search` (semantic search; ranked by relevance), `list` (filter-only browse, no semantic ranking — for audits or exact enumeration). Memory is the user's biography across sessions — durable identity, cross-project preferences, decisions with their reasoning, life context. Project-internal facts (code architecture, repo conventions) are NOT in memory — the agent reads the project's own files (source, the user's `AGENTS.md` / `CLAUDE.md` if any) directly when it needs that content.\n\n**All filters are optional and AND-combined; omit anything you aren't intentionally narrowing on.** Speculatively passing `from`, `outcome`, or a specific `type` is the #1 cause of empty results — most rows don't carry an `outcome`, and the user's actual data may not match the value you guessed. When unsure, start with just `verb` (+ `query` for search) and add filters only after you see what's there.".to_string(),
                // Memory ops are conversation primitives — the user's own
                // data being saved on their behalf — not workspace mutations.
                // Pinned at Chat so every session tier can use them without
                // a permission prompt, matching the "save silently" design
                // in system-prompt.toml's core_memory_block.
                tier: PermissionMode::Chat,
                args_schema: json!({
                    "type": "object",
                    "properties": {
                        "verb":     {"type": "string", "enum": ["get", "search", "list"], "description": "Read operation."},
                        "id":       {"type": "string", "description": "Required for verb=get. Fact UUID."},
                        "query":    {"type": "string", "description": "Required for verb=search. Natural-language description of what you're looking for."},
                        "contexts": {"type": "array", "items": {"type": "string"}, "description": "Filter to these scope tags (AND semantics). For verb=search, narrows ranked results; for verb=list, primary filter. Omit to skip."},
                        "type":     {"type": "string", "enum": ["fact", "preference", "decision", "tried", "fixed", "learned", "built"], "description": "Filter by fact type. Omit to return all types."},
                        "tier":     {"type": "string", "enum": ["core", "semantic", "episodic"], "description": "Memory category. `core` = always-on identity/style universals; `semantic` = curated retrieval pool; `episodic` = staging table (encoder writes here pre-promotion). **Omit to span all three.** Replaces the old `episodic: boolean` flag."},
                        "from":     {"type": "string", "enum": ["user", "agent", "derived"], "description": "**DEFAULT: do not pass.** Filter by origin. Pass only when the user explicitly asked to see rows from a specific origin (rare)."},
                        "outcome":  {"type": "string", "enum": ["positive", "negative", "neutral"], "description": "**DEFAULT: do not pass.** Filter by outcome. Almost no rows have `outcome=neutral`; passing it returns 0 rows even when the store has data. Pass only when the user explicitly asked to see only positive / negative outcomes."},
                        "since":    {"type": "string", "description": "RFC-3339 lower bound on effective timestamp. Omit to skip."},
                        "until":    {"type": "string", "description": "RFC-3339 upper bound (verb=list only). Omit to skip."},
                        "sort":     {"type": "string", "enum": ["newest", "oldest"], "description": "verb=list only. Defaults to newest."},
                        "limit":    {"type": "integer", "description": "Max rows. Defaults to 10 for search, 50 for list."},
                        "offset":   {"type": "integer", "description": "verb=list only. Skip this many rows in sort order."}
                    },
                    "required": ["verb"]
                }),
            },
            CapabilityTool {
                name: "Memory_write".to_string(),
                description: "Modify memory. Verb-dispatched: `add` (insert a new row), `update` (edit fields of an existing row by id), `delete` (hard-delete a single row by id). **READ BEFORE YOU WRITE — every `add` MUST be preceded by a `Memory_query` on the candidate's subject in the SAME turn.** If an equivalent row already exists (exact or a reworded restatement), SKIP the write — don't duplicate. If an existing row contradicts the candidate on the same subject (incompatible value, e.g. \"cat is male\" vs \"cat is female\"), call `AskUser` showing both rows with their dates; only write after the user's answer. Memory should grow with genuinely durable signal: cross-project user identity / goals (`fact`), commitment-language behavioral rules (`preference`), decisions whose reasoning is the retrieval value (`decision`), cross-project tech gotchas (`learned`). Don't store project-internal architecture, conventions, or implementation detail — drop those candidates entirely. Memory does NOT write to project files (`<project>/AGENTS.md`, `CLAUDE.md`, source, docs); those are user-curated, and the agent reads them directly when needed. **Append, don't overwrite**: when a new utterance contradicts or refines an existing row, `verb=add` a new row and let live retrieval reconcile by recency — never overwrite a user-stated fact on inference. Reserve `verb=update` for mechanical rephrasing of the same fact and `verb=delete` for explicit user requests to forget (or for the post-AskUser resolution of a contradiction). Bulk forget is not on this tool surface — handle it via the dashboard or by iterating verb=delete after explicit user confirmation.".to_string(),
                // Memory writes don't touch the workspace — they hit the
                // user's own daemon-backed store. Pinned at Chat so a
                // chat/read session can save "my cat's name is Xiaoman"
                // without a permission prompt; deletes are still bounded
                // by the spec ("explicit user requests to forget" only)
                // and surface a "Deleted: …" chip the user can object to.
                tier: PermissionMode::Chat,
                args_schema: json!({
                    "type": "object",
                    "properties": {
                        "verb":          {"type": "string", "enum": ["add", "update", "delete"], "description": "Write operation."},
                        "id":            {"type": "string", "description": "Required for verb=update and verb=delete. UUID of the target row."},
                        "content":       {"type": "string", "description": "Required for verb=add. Self-contained fact text. Optional for verb=update."},
                        "contexts":      {"type": "array", "items": {"type": "string"}, "description": "Scope tags (e.g. [\"cross-project\", \"music/piano\"]). Free-form; N:M with facts."},
                        "tags":          {"type": "array", "items": {"type": "string"}, "description": "Free-form metadata with prefix convention (e.g. \"topic:ui\", \"intent:goal\")."},
                        "type":          {"type": "string", "enum": ["fact", "preference", "decision", "tried", "fixed", "learned", "built"], "description": "verb=add/update. Prefer `fact` / `preference` / `decision` / `learned` for new writes; `tried` / `fixed` / `built` are deprecated."},
                        "tier":          {"type": "string", "enum": ["core", "semantic", "episodic"], "description": "verb=add/update. Destination memory category. `core` = always-injected identity/style universals (keep tiny); `semantic` (default) = curated retrieval pool; `episodic` = staging table (per-session encoder + dream consolidator write here; live agent saves stay on `semantic`). Replaces the old `episodic: boolean` flag."},
                        "from":          {"type": "string", "enum": ["user", "agent", "derived"], "description": "Origin. Pick `user` when the user said it directly, `derived` for cross-session synthesis, `agent` for agent observations."},
                        "outcome":       {"type": "string", "enum": ["positive", "negative", "neutral"], "description": "Only meaningful for `tried` / `fixed` / `decision`. Omit for `fact` / `preference` / `learned`."},
                        "clear_outcome": {"type": "boolean", "description": "verb=update only. Clear outcome to null."},
                        "cwd":           {"type": "string", "description": "Working directory where the fact was produced. Pass through from the source session, don't substitute your own."},
                        "clear_cwd":     {"type": "boolean", "description": "verb=update only. Clear cwd to null."},
                        "host":          {"type": "string", "description": "Writing host (`linggen` / `claude-code` / `codex` / …). Identifies which tool runtime committed the row. Distinct from `from` (which says whether content came from user/agent/derived)."},
                        "clear_host":    {"type": "boolean", "description": "verb=update only. Clear host to null."},
                        "occurred_at":   {"type": "string", "description": "verb=add only. RFC-3339 timestamp of the described event (e.g. \"2026-04-27T16:00:00Z\"). **Omit entirely if unknown** — do not pass empty strings, partial dates, or null. Date-only \"YYYY-MM-DD\" is also accepted (interpreted as midnight UTC)."},
                        "source_session":{"type": "string", "description": "verb=add only. Opaque session id the fact was extracted from."},
                        "skip_dedup":    {"type": "boolean", "description": "verb=add only. Skip server-side merge-into-near-duplicate. Set to true when running your own dedup pass."}
                    },
                    "required": ["verb"]
                }),
            },
        ],
    }
}

// ── Lookup helpers ─────────────────────────────────────────────────────

/// Find the capability that owns the given tool name. Returns the
/// capability's name + a ref to its tool descriptor. The returned
/// references live as long as `CAPABILITIES` — which is a `LazyLock`
/// in static position, i.e. program-lifetime.
pub fn capability_for_tool(tool_name: &str) -> Option<(&'static str, &'static CapabilityTool)> {
    CAPABILITIES.iter().find_map(|cap| {
        cap.tools
            .iter()
            .find(|t| t.name == tool_name)
            .map(|t| (cap.name.as_str(), t))
    })
}

/// Iterate every capability tool across every registered capability.
pub fn all_capability_tools() -> impl Iterator<Item = (&'static str, &'static CapabilityTool)> {
    CAPABILITIES
        .iter()
        .flat_map(|cap| cap.tools.iter().map(move |t| (cap.name.as_str(), t)))
}

/// Permission tier for a capability tool by name.
pub fn tool_tier(tool_name: &str) -> Option<PermissionMode> {
    capability_for_tool(tool_name).map(|(_, t)| t.tier)
}

/// Is this tool name a capability tool (as opposed to a built-in or a
/// skill-unique tool)?
pub fn is_capability_tool(tool_name: &str) -> bool {
    capability_for_tool(tool_name).is_some()
}

/// Render a capability tool's arg schema to the legacy tool-schema
/// format used by `full_tool_schema_entries` — `{"name", "args":{k:"type"},
/// "returns", "notes"}`. The "args" here are compact type strings derived
/// from the JSON Schema (`string` / `string?` / `string[]` / `string[]?`
/// / `integer` / etc.).
pub fn legacy_schema_entry(tool: &CapabilityTool) -> Value {
    let mut args = serde_json::Map::new();
    let required: Vec<String> = tool
        .args_schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    if let Some(props) = tool.args_schema.get("properties").and_then(|p| p.as_object()) {
        for (name, spec) in props {
            let is_required = required.iter().any(|r| r == name);
            let ty = spec.get("type").and_then(|v| v.as_str()).unwrap_or("string");
            let compact = match ty {
                "array" => {
                    let item_ty = spec
                        .get("items")
                        .and_then(|it| it.get("type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("string");
                    format!("{item_ty}[]")
                }
                other => other.to_string(),
            };
            let with_opt = if is_required {
                compact
            } else {
                format!("{compact}?")
            };
            args.insert(name.clone(), Value::String(with_opt));
        }
    }
    let mut entry = json!({
        "name": tool.name,
        "args": args,
    });
    if !tool.description.is_empty() {
        entry["notes"] = Value::String(tool.description.clone());
    }
    entry
}

/// OpenAI-compatible function schema for native tool calling.
pub fn oai_schema_entry(tool: &CapabilityTool) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.args_schema,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_capability_registered_with_two_tools() {
        let mem = CAPABILITIES
            .iter()
            .find(|c| c.name == "memory")
            .expect("memory capability present");
        assert_eq!(mem.tools.len(), 2);
        let names: Vec<&str> = mem.tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"Memory_query"));
        assert!(names.contains(&"Memory_write"));
    }

    #[test]
    fn capability_for_tool_finds_memory_tools() {
        let (cap_name, tool) =
            capability_for_tool("Memory_query").expect("Memory_query is a capability tool");
        assert_eq!(cap_name, "memory");
        assert_eq!(tool.name, "Memory_query");
        assert_eq!(tool.tier, PermissionMode::Chat);
    }

    #[test]
    fn capability_for_tool_misses_for_builtins() {
        assert!(capability_for_tool("Read").is_none());
        assert!(capability_for_tool("Bash").is_none());
        assert!(capability_for_tool("Unknown").is_none());
        // Old per-verb names are no longer capability tools.
        assert!(capability_for_tool("Memory_search").is_none());
        assert!(capability_for_tool("Memory_add").is_none());
        assert!(capability_for_tool("Memory_forget").is_none());
    }

    #[test]
    fn tool_tiers_match_canonical_contract() {
        // Both memory tools are pinned at Chat so any session tier
        // (chat/read/edit/admin) can use them without a permission prompt.
        // See the per-tool comments in `memory_capability` for why.
        assert_eq!(tool_tier("Memory_query"), Some(PermissionMode::Chat));
        assert_eq!(tool_tier("Memory_write"), Some(PermissionMode::Chat));
    }

    #[test]
    fn legacy_schema_entry_compacts_types() {
        let tool = capability_for_tool("Memory_query").unwrap().1;
        let entry = legacy_schema_entry(tool);
        assert_eq!(entry["name"], "Memory_query");
        assert_eq!(entry["args"]["verb"], "string");
        assert_eq!(entry["args"]["contexts"], "string[]?");
        assert_eq!(entry["args"]["limit"], "integer?");
    }

    #[test]
    fn oai_schema_wraps_as_function() {
        let tool = capability_for_tool("Memory_write").unwrap().1;
        let entry = oai_schema_entry(tool);
        assert_eq!(entry["type"], "function");
        assert_eq!(entry["function"]["name"], "Memory_write");
        let required = entry["function"]["parameters"]["required"]
            .as_array()
            .unwrap();
        assert!(required.iter().any(|v| v == "verb"));
    }
}

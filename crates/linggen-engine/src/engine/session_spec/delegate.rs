//! An `agent_run` delegate's toolset: kept in its session's params, so a
//! continued session keeps the toolset it was started with.

use crate::state_fs::sessions::SessionMeta;
use std::collections::HashSet;

/// The delegate's tool allowlist. Read / search / memory / browser only —
/// no Bash, Write, Edit, or Task, so the read-only boundary can't be worked
/// around (e.g. `echo > file` via Bash). Browser mutations still pass the
/// extension's own permission prompt.
const READ_TOOLS: &[&str] = &[
    "Read",
    "Grep",
    "Glob",
    "WebSearch",
    "ImageSearch",
    "VideoSearch",
    "WebFetch",
    // The memory server by name, expanded to the tools it advertises — so
    // this list doesn't have to be re-edited when ling-mem gains a verb.
    "mcp__memory",
    "Skill",
    // Its checklist shows in the session, as in any chat (it writes nothing).
    "UpdatePlan",
    // Pictures, as in any chat; ChatGPT's lane when the caller asks for it.
    "GenerateImage",
    "Browser_navigate",
    "Browser_readPage",
    "Browser_screenshot",
    "Browser_click",
    "Browser_type",
    "Browser_key",
    "Browser_scroll",
    "Browser_wait",
    "Browser_readConsole",
    "Browser_tabs",
];

/// What a write run adds to the read-only toolset.
const WRITE_TOOLS: &[&str] = &["Bash", "Write", "Edit"];

/// The session param that keeps a delegate's toolset: `read` or `write`.
pub const TOOLS_PARAM: &str = "agent_run_tools";

/// The delegate's toolset: read / search / memory / browser, plus Bash,
/// Write and Edit for a write run.
pub fn tool_scope(write: bool) -> Option<HashSet<String>> {
    let extra: &[&str] = if write { WRITE_TOOLS } else { &[] };
    let declared: Vec<String> = READ_TOOLS
        .iter()
        .chain(extra)
        .map(|s| s.to_string())
        .collect();
    crate::engine::tool_scope::compute_tool_scope(&declared)
}

/// The toolset a delegate's session keeps: `Some(write)`, or `None` for a
/// session that kept none.
pub fn kept_toolset(meta: &SessionMeta) -> Option<bool> {
    meta.params.get(TOOLS_PARAM).map(|kind| kind == "write")
}

/// The param value a new delegate's session keeps.
pub fn toolset_param(write: bool) -> &'static str {
    if write {
        "write"
    } else {
        "read"
    }
}

//! `memory_notes` — Linggen's own memory (Memory V2) in a Linggen session:
//! the index, a note in full, a search, and the one writer. The ops and
//! their answers are `own_memory::call`, which the MCP door's tool of the
//! same name calls too; this is the session's face of it. Offered only
//! where `[memory] linggen = "v2"` (`AgentEngine::is_withheld`).

use super::super::{ToolCall, ToolResult, Tools};
use super::Tool;
use crate::engine::permission::PermissionMode;
use crate::own_memory::{self, Audience, Caller, Store};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct MemoryNotesTool;

#[async_trait]
impl Tool for MemoryNotesTool {
    fn name(&self) -> &'static str {
        own_memory::TOOL
    }

    fn description(&self) -> &'static str {
        own_memory::spec::DESCRIPTION
    }

    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }

    /// A write keeps what the person said past this conversation, and a
    /// private app's talk must not land there.
    fn place_reach(&self) -> crate::engine::tools::PlaceReach {
        crate::engine::tools::PlaceReach::CarriesOut
    }

    fn args_schema(&self) -> Value {
        own_memory::spec::session_schema()
    }

    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": own_memory::TOOL,
            "args": {"op": "index|read|search|titles|write", "id": "string?", "words": "string?", "summary": "string?"},
            "returns": "the answer as JSON, or the reason it was refused",
            "notes": "Linggen's own notes about the person and the work."
        })
    }

    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let op = call.args.get("op").and_then(Value::as_str).unwrap_or_default().to_string();
        let store = Store::this_machine().ok_or_else(|| anyhow::anyhow!("memory_notes: this machine has no home folder"))?;
        let caller = Caller {
            audience: tools.memory_audience.unwrap_or(Audience::Work),
            source: if tools.is_companion() { "yinyue" } else { "linggen" }.to_string(),
            cwd: Some(tools.cwd()),
        };
        let now = crate::util::now_ts_secs() as i64;
        Ok(match own_memory::call(&store, &caller, &op, &call.args, now) {
            Ok(answer) => ToolResult::Success(answer.to_string()),
            Err(why) => ToolResult::Success(format!("not done: {why}")),
        })
    }
}

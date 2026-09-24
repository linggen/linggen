//! File reads: Glob, Read, Grep.

use super::super::file_tools::{ListFilesArgs, ReadFileArgs};
use super::super::search_exec::SearchArgs;
use super::super::{ToolCall, ToolResult, Tools};
use super::Tool;
use crate::engine::permission::PermissionMode;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

pub struct GlobTool;
#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &'static str {
        "Glob"
    }
    fn cacheable(&self) -> bool {
        true
    }
    // A filesystem walk. The default 5 min is meaningless here: a glob that
    // has not answered in a minute is walking somewhere it should not be.
    fn max_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs(60))
    }
    fn description(&self) -> &'static str {
        "Find files by glob pattern. Returns matching file paths sorted by modification time."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "globs": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Glob patterns to match (e.g. [\"**/*.rs\", \"src/**/*.ts\"])"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return"
                }
            },
            "required": ["globs"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Glob",
            "args": {"globs": "string[]?", "max_results": "number?"},
            "returns": "string[]",
            "notes": "Glob pattern aliases accepted: globs, pattern, glob."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: ListFilesArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for Glob: {}", e))?;
        tools.list_files(args).await
    }
}

pub struct ReadTool;
#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &'static str {
        "Read"
    }
    fn cacheable(&self) -> bool {
        true
    }
    fn description(&self) -> &'static str {
        "Read a file's contents. Path can be relative (resolved from workspace root) or absolute. Always read a file before modifying it."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path to read (relative to workspace root, or absolute)"
                },
                "max_bytes": {
                    "type": "integer",
                    "description": "Maximum bytes to read (default: entire file)"
                },
                "line_range": {
                    "type": "array",
                    "items": {"type": "integer"},
                    "minItems": 2,
                    "maxItems": 2,
                    "description": "Line range [start, end] (1-based, inclusive)"
                }
            },
            "required": ["path"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Read",
            "args": {"path": "string", "max_bytes": "number?", "line_range": "[number,number]?"},
            "returns": "{path,content,truncated}",
            "notes": "Path aliases accepted: path, file, filepath."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: ReadFileArgs = serde_json::from_value(call.args).map_err(|e| {
            anyhow::anyhow!(
                "invalid args for Read: {}. Expected keys: path|max_bytes|line_range",
                e
            )
        })?;
        tools.read_file(args).await
    }
}

pub struct GrepTool;
#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &'static str {
        "Grep"
    }
    fn cacheable(&self) -> bool {
        true
    }
    // Same reasoning as Glob — it walks the tree.
    fn max_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs(60))
    }
    fn description(&self) -> &'static str {
        "Search file contents using regex. Returns matching lines with file path, line number, and snippet."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Read
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Regex pattern to search for"
                },
                "globs": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "File glob patterns to search within (e.g. [\"**/*.rs\"])"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of matches to return"
                }
            },
            "required": ["query"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Grep",
            "args": {"query": "string", "globs": "string[]?", "max_results": "number?"},
            "returns": "{matches:[{path,line,snippet}]}",
            "notes": "Query aliases accepted: query, path, file, filepath."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let args: SearchArgs = serde_json::from_value(call.args)
            .map_err(|e| anyhow::anyhow!("invalid args for Grep: {}", e))?;
        tools.search_rg(args).await
    }
}

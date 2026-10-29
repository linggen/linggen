//! Bash.

use super::super::search_exec::RunCommandArgs;
use super::super::{ToolCall, ToolResult, Tools};
use super::Tool;
use crate::engine::permission::PermissionMode;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;

pub struct BashTool;
#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &'static str {
        "Bash"
    }
    // The user asked for this command; builds and downloads run long, and the
    // shell layer bounds itself.
    fn max_duration(&self) -> Option<Duration> {
        None
    }
    fn description(&self) -> &'static str {
        "Run a shell command via sh -c on this machine — the person's own computer, the one you run on. Working directory persists across calls (cd is remembered). Use for builds, tests, git, and reading the machine's own live state (memory, disks, processes, network, settings). Prefer dedicated tools (Read, Glob, Grep) over Bash equivalents."
    }
    fn tier(&self) -> PermissionMode {
        PermissionMode::Admin
    }
    fn args_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "cmd": {"type": "string", "description": "Shell command to execute"},
                "description": {"type": "string", "description": "What the command does, in a few plain words (\"Build the UI\", \"Run the engine tests\") — the chat shows it while the command runs"},
                "timeout_ms": {"type": "integer", "description": "Timeout in milliseconds (default: 30000)"},
                "run_in_background": {"type": "boolean", "description": "Start it beside the chat and return at once with a task id — for servers, watches and long builds. You are told in a new turn when it ends; TaskOutput reads its output, TaskStop stops it."}
            },
            "required": ["cmd"]
        })
    }
    fn legacy_schema_entry(&self) -> Value {
        json!({
            "name": "Bash",
            "args": {"cmd": "string", "description": "string?", "timeout_ms": "number?", "run_in_background": "bool?"},
            "returns": "{exit_code,stdout,stderr}",
            "notes": "Runs shell commands via sh -c. Permission required in ask mode. Command alias accepted: command."
        })
    }
    async fn execute(&self, tools: &Tools, call: ToolCall) -> Result<ToolResult> {
        let mut args: RunCommandArgs = serde_json::from_value(call.args).map_err(|e| {
            anyhow::anyhow!(
                "invalid args for Bash: {}. Expected keys: cmd|timeout_ms|run_in_background",
                e
            )
        })?;
        if args.run_in_background {
            return background_shell(tools, &args.cmd).await;
        }
        // Bash is the only tool with mid-execution cancellation: register a
        // cancel flag against the block_id so an in-flight `kill` from the
        // UI can interrupt the child process.
        if let (Some(bid), Some(mgr)) = (&call.block_id, &tools.manager) {
            args.cancel_flag = Some(mgr.register_tool_cancel_flag(bid));
        }
        let result = tools.run_command(args).await;
        if let (Some(bid), Some(mgr)) = (&call.block_id, &tools.manager) {
            mgr.clear_tool_cancel_flag(bid);
        }
        result
    }
}

/// Start the command as a background task of the session and return at once.
async fn background_shell(tools: &Tools, cmd: &str) -> Result<ToolResult> {
    let (Some(manager), Some(session)) = (&tools.manager, &tools.session_id) else {
        anyhow::bail!("run_in_background needs a session; run it in the foreground instead");
    };
    let agent = tools.agent_id.clone().unwrap_or_else(|| "ling".into());
    let task = crate::engine::background::spawn_shell(manager, session, &agent, cmd, &tools.cwd()).await?;
    Ok(ToolResult::Success(format!(
        "Started in the background as task {}. Output: {}. It runs beside the chat; \
         you will be told in a new turn when it ends. Read its output with TaskOutput, \
         stop it with TaskStop.",
        task.id,
        task.output_file.as_deref().unwrap_or("")
    )))
}

/// Ling runs on the person's own computer. Every text that frames Bash says
/// so, and says Bash reads that machine's live state: when Bash was "for
/// build/test/git" and nothing said whose machine this was, a plain "should I
/// worry about my Mac's memory?" went looking for a dedicated tool, found
/// none, and answered "I can't see your Mac" in about half the runs
/// (2026-10-22 A/B).
#[cfg(test)]
mod machine_tests {
    use super::*;
    use crate::prompts::{keys, PromptStore};

    const OWN: &str = "the person's own computer";
    const STATE: &str = "memory, disks, processes, network, settings";

    #[test]
    fn bash_is_how_ling_reads_the_machine_it_runs_on() {
        let bash = BashTool.description();
        assert!(bash.contains(OWN) && bash.contains(STATE), "{bash}");

        let store = PromptStore::load(None);
        let env = store.get(keys::SYSTEM_ENVIRONMENT_BLOCK).unwrap();
        assert!(env.contains(OWN) && env.contains(STATE), "{env}");
        for key in ["response-format.native", "response-format.default"] {
            let rules = store.get(key).unwrap();
            assert!(!rules.contains("Reserve Bash for build/test/git"), "{key}");
            assert!(rules.contains(STATE), "{key} never names the machine's state");
        }
    }
}

/// Bash may say what its command does: the chat shows that while it runs
/// (ui/src/lib/bashLabel.mts). It is for the chat only; the call still runs.
#[cfg(test)]
mod description_tests {
    use super::*;

    #[test]
    fn bash_takes_an_optional_description() {
        let schema = BashTool.args_schema();
        assert_eq!(schema["properties"]["description"]["type"], "string");
        assert_eq!(schema["required"], json!(["cmd"]));
        let args: RunCommandArgs =
            serde_json::from_value(json!({"cmd": "ls", "description": "List files"})).unwrap();
        assert_eq!(args.cmd, "ls");
    }
}

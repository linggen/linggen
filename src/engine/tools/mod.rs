mod file_tools;
mod search_exec;
mod write_tools;
mod delegation;
mod tool_helpers;

pub use tool_helpers::canonical_tool_name;
pub(crate) use tool_helpers::full_tool_schema_entries;
pub(crate) use tool_helpers::{normalize_tool_args, summarize_tool_args};
pub(crate) use delegation::{run_delegation, TaskArgs};

use crate::agent_manager::AgentManager;
use crate::config::AgentPolicy;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};
use tracing::debug;

// ── Helpers ─────────────────────────────────────────────────────────────

/// Run an async future to completion from synchronous code.
///
/// On a multi-threaded tokio runtime worker: uses `block_in_place` + `block_on` so the
/// runtime can reclaim the worker thread while blocking.
/// Otherwise (e.g., parallel batch threads without a runtime): creates a temporary
/// `current_thread` runtime to drive the future.
pub(crate) fn block_on_async<F: std::future::Future>(future: F) -> F::Output {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(future)),
        Err(_) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to create async runtime for tool execution");
            rt.block_on(future)
        }
    }
}

/// Check if a hostname falls in the RFC 1918 172.16.0.0/12 range (172.16.x.x – 172.31.x.x).
fn is_rfc1918_172(host: &str) -> bool {
    if let Some(rest) = host.strip_prefix("172.") {
        if let Some(second_octet) = rest.split('.').next() {
            if let Ok(n) = second_octet.parse::<u8>() {
                return (16..=31).contains(&n);
            }
        }
    }
    false
}

// ── Public types ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub tool: String,
    pub args: Value,
}

#[derive(Debug, Serialize)]
pub enum ToolResult {
    FileList(Vec<String>),
    FileContent {
        path: String,
        content: String,
        truncated: bool,
    },
    SearchMatches(Vec<SearchMatch>),
    CommandOutput {
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    Screenshot {
        url: String,
        base64: String,
    },
    Success(String),
    LockResult {
        acquired: Vec<(String, String)>,
        denied: Vec<String>,
    },
    AgentOutcome(crate::engine::AgentOutcome),
    WebSearchResults {
        query: String,
        results: Vec<super::web_search::WebSearchResult>,
    },
    WebFetchContent {
        url: String,
        content: String,
        content_type: String,
        truncated: bool,
    },
    AskUserResponse {
        answers: Vec<AskUserAnswer>,
    },
}

// ── AskUser types ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AskUserQuestion {
    pub question: String,
    pub header: String,
    pub options: Vec<AskUserOption>,
    #[serde(default)]
    pub multi_select: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AskUserOption {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub preview: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AskUserArgs {
    questions: Vec<AskUserQuestion>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AskUserAnswer {
    pub question_index: usize,
    pub selected: Vec<String>,
    pub custom_text: Option<String>,
}

/// Bridge between the synchronous tool executor and the async server state,
/// allowing the AskUser tool to emit SSE events and block on user responses.
pub struct AskUserBridge {
    pub events_tx: broadcast::Sender<crate::server::ServerEvent>,
    pub pending: Arc<Mutex<HashMap<String, PendingAskUser>>>,
}

pub struct PendingAskUser {
    pub agent_id: String,
    pub sender: tokio::sync::oneshot::Sender<Vec<AskUserAnswer>>,
}

#[derive(Debug, Serialize)]
pub struct SearchMatch {
    pub path: String,
    pub line: usize,
    pub snippet: String,
}

/// Sender for streaming tool progress lines (tool_name, stream_name, line).
/// Uses tokio's unbounded channel so the sender works in sync contexts
/// and the receiver can be held across async .await points.
pub type ToolProgressSender = tokio::sync::mpsc::UnboundedSender<(String, String, String)>;

// ── Tools struct ────────────────────────────────────────────────────────

pub struct Tools {
    root: PathBuf,
    manager: Option<Arc<AgentManager>>,
    agent_id: Option<String>,
    delegation_depth: usize,
    max_delegation_depth: usize,
    run_id: Option<String>,
    agent_policy: Option<AgentPolicy>,
    memory_dir: Option<PathBuf>,
    ask_user_bridge: Option<Arc<AskUserBridge>>,
    progress_tx: Option<ToolProgressSender>,
}

impl Tools {
    pub fn new(root: PathBuf) -> Result<Self> {
        Ok(Self {
            root,
            manager: None,
            agent_id: None,
            delegation_depth: 0,
            max_delegation_depth: 2,
            run_id: None,
            agent_policy: None,
            memory_dir: None,
            ask_user_bridge: None,
            progress_tx: None,
        })
    }

    pub fn set_context(
        &mut self,
        manager: Arc<AgentManager>,
        agent_id: String,
    ) {
        self.manager = Some(manager);
        self.agent_id = Some(agent_id);
    }

    pub fn set_delegation_depth(&mut self, depth: usize) {
        self.delegation_depth = depth;
    }

    pub fn set_max_delegation_depth(&mut self, max_depth: usize) {
        self.max_delegation_depth = max_depth;
    }

    pub fn delegation_depth(&self) -> usize {
        self.delegation_depth
    }

    pub fn max_delegation_depth(&self) -> usize {
        self.max_delegation_depth
    }

    pub fn set_policy(&mut self, policy: Option<AgentPolicy>) {
        self.agent_policy = policy;
    }

    pub fn set_run_id(&mut self, run_id: Option<String>) {
        self.run_id = run_id;
    }

    pub fn set_memory_dir(&mut self, dir: PathBuf) {
        self.memory_dir = Some(dir);
    }

    pub fn memory_dir(&self) -> Option<&PathBuf> {
        self.memory_dir.as_ref()
    }

    pub fn set_ask_user_bridge(&mut self, bridge: Arc<AskUserBridge>) {
        self.ask_user_bridge = Some(bridge);
    }

    pub fn set_progress_tx(&mut self, tx: ToolProgressSender) {
        self.progress_tx = Some(tx);
    }

    pub fn ask_user_bridge(&self) -> Option<&Arc<AskUserBridge>> {
        self.ask_user_bridge.as_ref()
    }

    pub fn get_manager(&self) -> Option<Arc<AgentManager>> {
        self.manager.clone()
    }

    pub(crate) fn workspace_root(&self) -> &Path {
        &self.root
    }

    // ── Execute dispatcher ──────────────────────────────────────────────

    pub fn execute(&self, call: ToolCall) -> Result<ToolResult> {
        let normalized_args = normalize_tool_args(&call.tool, call.args);
        debug!(
            "Executing tool: {} args={}",
            call.tool,
            summarize_tool_args(&call.tool, &normalized_args)
        );
        match call.tool.as_str() {
            "Glob" => {
                let args: file_tools::ListFilesArgs = serde_json::from_value(normalized_args)
                    .map_err(|e| anyhow::anyhow!("invalid args for Glob: {}", e))?;
                self.list_files(args)
            }
            "Read" => {
                let args: file_tools::ReadFileArgs = serde_json::from_value(normalized_args).map_err(|e| {
                    anyhow::anyhow!(
                        "invalid args for Read: {}. Expected keys: path|max_bytes|line_range",
                        e
                    )
                })?;
                self.read_file(args)
            }
            "Grep" => {
                let args: search_exec::SearchArgs = serde_json::from_value(normalized_args)
                    .map_err(|e| anyhow::anyhow!("invalid args for Grep: {}", e))?;
                self.search_rg(args)
            }
            "Bash" => {
                let args: search_exec::RunCommandArgs =
                    serde_json::from_value(normalized_args).map_err(|e| {
                        anyhow::anyhow!(
                            "invalid args for Bash: {}. Expected keys: cmd|timeout_ms",
                            e
                        )
                    })?;
                self.run_command(args)
            }
            "capture_screenshot" => {
                let args: file_tools::CaptureScreenshotArgs = serde_json::from_value(normalized_args)
                    .map_err(|e| anyhow::anyhow!("invalid args for capture_screenshot: {}", e))?;
                self.capture_screenshot(args)
            }
            "Write" => {
                let args: write_tools::WriteFileArgs = serde_json::from_value(normalized_args).map_err(|e| {
                    anyhow::anyhow!("invalid args for Write: {}. Expected keys: path|content", e)
                })?;
                self.write_file(args)
            }
            "Edit" => {
                let args: write_tools::EditFileArgs = serde_json::from_value(normalized_args).map_err(|e| {
                    anyhow::anyhow!(
                        "invalid args for Edit: {}. Expected keys: path|old_string|new_string|replace_all?",
                        e
                    )
                })?;
                self.edit_file(args)
            }
            "lock_paths" => {
                let args: write_tools::LockPathsArgs = serde_json::from_value(normalized_args)
                    .map_err(|e| anyhow::anyhow!("invalid args for lock_paths: {}", e))?;
                self.lock_paths(args)
            }
            "unlock_paths" => {
                let args: write_tools::UnlockPathsArgs = serde_json::from_value(normalized_args)
                    .map_err(|e| anyhow::anyhow!("invalid args for unlock_paths: {}", e))?;
                self.unlock_paths(args)
            }
            "Task" => {
                let args: delegation::TaskArgs = serde_json::from_value(normalized_args)
                    .map_err(|e| anyhow::anyhow!("invalid args for Task: {}", e))?;
                self.task(args)
            }
            "WebSearch" => {
                let args: delegation::WebSearchArgs = serde_json::from_value(normalized_args)
                    .map_err(|e| anyhow::anyhow!("invalid args for WebSearch: {}", e))?;
                let max = args.max_results.unwrap_or(5).min(10);
                let results =
                    block_on_async(super::web_search::web_search(&args.query, max))?;
                Ok(ToolResult::WebSearchResults {
                    query: args.query,
                    results,
                })
            }
            "WebFetch" => {
                let args: delegation::WebFetchArgs = serde_json::from_value(normalized_args)
                    .map_err(|e| anyhow::anyhow!("invalid args for WebFetch: {}", e))?;
                let result =
                    block_on_async(super::web_fetch::fetch_url(&args.url, args.max_bytes))?;
                Ok(ToolResult::WebFetchContent {
                    url: result.url,
                    content: result.content,
                    content_type: result.content_type,
                    truncated: result.truncated,
                })
            }
            "Skill" => {
                let args: delegation::SkillArgs = serde_json::from_value(normalized_args)
                    .map_err(|e| anyhow::anyhow!("invalid args for Skill: {}", e))?;
                self.invoke_skill(args)
            }
            "AskUser" => {
                let args: AskUserArgs = serde_json::from_value(normalized_args)
                    .map_err(|e| anyhow::anyhow!("invalid args for AskUser: {}", e))?;

                // Validate question count.
                if args.questions.is_empty() || args.questions.len() > 4 {
                    anyhow::bail!("AskUser requires 1-4 questions, got {}", args.questions.len());
                }
                for (i, q) in args.questions.iter().enumerate() {
                    if q.options.len() < 2 || q.options.len() > 4 {
                        anyhow::bail!(
                            "AskUser question {} requires 2-4 options, got {}",
                            i, q.options.len()
                        );
                    }
                }

                // Sub-agents cannot use AskUser.
                if self.delegation_depth > 0 {
                    return Ok(ToolResult::Success(
                        "AskUser is not available for delegated sub-agents. Return your question to the parent agent instead.".to_string()
                    ));
                }

                let bridge = match &self.ask_user_bridge {
                    Some(b) => Arc::clone(b),
                    None => {
                        return Ok(ToolResult::Success(
                            "AskUser is not available in CLI mode.".to_string()
                        ));
                    }
                };

                let question_id = uuid::Uuid::new_v4().to_string();
                let agent_id = self.agent_id.clone().unwrap_or_default();

                // Emit SSE event to push the question to the UI.
                let _ = bridge.events_tx.send(crate::server::ServerEvent::AskUser {
                    agent_id: agent_id.clone(),
                    question_id: question_id.clone(),
                    questions: args.questions,
                });

                // Create a oneshot channel and register it for the response endpoint.
                let (tx, rx) = tokio::sync::oneshot::channel();
                block_on_async(async {
                    bridge.pending.lock().await.insert(
                        question_id.clone(),
                        PendingAskUser { agent_id, sender: tx },
                    );
                });

                // Block until the user responds or timeout (5 minutes).
                let response = block_on_async(async {
                    tokio::time::timeout(Duration::from_secs(300), rx).await
                });

                // Cleanup: remove from pending map regardless of outcome.
                block_on_async(async {
                    bridge.pending.lock().await.remove(&question_id);
                });

                match response {
                    Ok(Ok(answers)) => Ok(ToolResult::AskUserResponse { answers }),
                    Ok(Err(_)) => Ok(ToolResult::Success(
                        "AskUser cancelled: the question was dismissed.".to_string(),
                    )),
                    Err(_) => Ok(ToolResult::Success(
                        "AskUser timed out: no response within 5 minutes.".to_string(),
                    )),
                }
            }
            "ExitPlanMode" => Ok(ToolResult::Success(
                "Plan submitted for review.".to_string(),
            )),
            _ => anyhow::bail!("unknown tool: {}", call.tool),
        }
    }
}

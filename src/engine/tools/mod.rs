mod builtin;
pub use builtin::builtin_tier;

mod file_tools;
pub(crate) mod json_schema;
mod search_exec;
mod write_tools;
mod delegation;
mod tool_helpers;

pub use tool_helpers::canonical_tool_name;
pub use search_exec::find_git_root as search_exec_find_git_root;
pub(crate) use tool_helpers::full_tool_schema_entries;
pub(crate) use tool_helpers::{normalize_tool_args, summarize_tool_args};
pub(crate) use delegation::{run_delegation, TaskArgs};

use crate::agent_manager::AgentManager;
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
    pub block_id: Option<String>,
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
/// allowing the AskUser tool to emit events and block on user responses.
pub struct AskUserBridge {
    pub events_tx: broadcast::Sender<crate::server::ServerEvent>,
    pub pending: Arc<Mutex<HashMap<String, PendingAskUser>>>,
    pub session_id: Option<String>,
}

pub struct PendingAskUser {
    pub agent_id: String,
    pub questions: Vec<AskUserQuestion>,
    pub sender: tokio::sync::oneshot::Sender<Vec<AskUserAnswer>>,
    pub session_id: Option<String>,
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

#[derive(Clone)]
pub struct Tools {
    root: PathBuf,
    /// Per-session working directory. Key = session_id, value = cwd.
    /// Sessions without an entry default to `root`.
    cwd_by_session: Arc<std::sync::Mutex<std::collections::HashMap<String, PathBuf>>>,
    manager: Option<Arc<AgentManager>>,
    agent_id: Option<String>,
    delegation_depth: usize,
    max_delegation_depth: usize,
    run_id: Option<String>,
    ask_user_bridge: Option<Arc<AskUserBridge>>,
    progress_tx: Option<ToolProgressSender>,
    prompt_store: Option<Arc<crate::prompts::PromptStore>>,
    pub(crate) session_id: Option<String>,
    /// Session policy propagated to subagents via delegation.
    /// Set by the parent engine; applied to subagent engines after spawn.
    pub(crate) session_policy: Option<super::session_policy::SessionPolicy>,
    /// Parent engine's path-mode grants, propagated to subagents via delegation.
    /// Mirrors `engine.session_permissions.path_modes` and is re-synced by the
    /// caller before each tool dispatch. Subagents inherit these so skill-
    /// granted paths (e.g. `~/.linggen`) don't require re-prompting inside a
    /// delegated run.
    pub(crate) parent_path_modes: Vec<super::permission::PathMode>,
    /// Parent engine's `interactive` flag. Propagated to subagents so that a
    /// non-interactive parent (mission, proxy consumer) yields a non-interactive
    /// child — prompts in unattended runs deadlock.
    pub(crate) parent_interactive: bool,
}

impl Tools {
    pub fn new(root: PathBuf) -> Result<Self> {
        let cwd_by_session = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        Ok(Self {
            root,
            cwd_by_session,
            manager: None,
            agent_id: None,
            delegation_depth: 0,
            max_delegation_depth: 2,
            run_id: None,
            ask_user_bridge: None,
            progress_tx: None,
            prompt_store: None,
            session_id: None,
            session_policy: None,
            parent_path_modes: Vec::new(),
            parent_interactive: true,
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

    pub fn set_run_id(&mut self, run_id: Option<String>) {
        self.run_id = run_id;
    }



    pub fn set_ask_user_bridge(&mut self, bridge: Arc<AskUserBridge>) {
        self.ask_user_bridge = Some(bridge);
    }

    pub fn set_progress_tx(&mut self, tx: ToolProgressSender) {
        self.progress_tx = Some(tx);
    }

    pub fn set_prompt_store(&mut self, store: Arc<crate::prompts::PromptStore>) {
        self.prompt_store = Some(store);
    }

    pub fn set_session_id(&mut self, session_id: Option<String>) {
        self.session_id = session_id;
    }

    /// Render a prompt template with fallback.
    pub fn prompt(&self, key: &str, vars: &[(&str, &str)]) -> String {
        match &self.prompt_store {
            Some(store) => store.render_or_fallback(key, vars),
            None => format!("[missing prompt: {}]", key),
        }
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

    /// Update the workspace root (e.g. when the agent enters a new git project).
    /// This makes Read/Write/Edit/Glob/Grep resolve relative paths from the new root.
    pub fn set_workspace_root(&mut self, new_root: PathBuf) {
        self.root = new_root;
    }

    pub fn cwd(&self) -> PathBuf {
        let map = self.cwd_by_session.lock().unwrap();
        if let Some(sid) = &self.session_id {
            map.get(sid).cloned().unwrap_or_else(|| self.root.clone())
        } else {
            self.root.clone()
        }
    }

    /// Seed the per-session cwd if not already set. Used when a session-bound
    /// skill activates — the skill's permission grant usually targets a
    /// specific path (e.g. ~/.linggen), and aligning session_cwd with that
    /// path lets Bash permission checks resolve to the granted mode.
    /// Callers should pass an absolute, expanded path.
    pub fn seed_session_cwd_if_unset(&self, path: PathBuf) {
        let Some(sid) = &self.session_id else { return };
        let mut map = self.cwd_by_session.lock().unwrap();
        map.entry(sid.clone()).or_insert(path);
    }

    // ── Execute dispatcher ──────────────────────────────────────────────

    /// Run a tool call by looking up the tool in the [`builtin`] registry
    /// and dispatching to its `execute` impl. Unknown tool names fall
    /// through to `ToolRegistry::execute`, which handles capability tools
    /// and skill tools.
    /// Run a synchronous, CPU-bound or filesystem-blocking closure on the
    /// blocking pool. The closure receives a clone of `Tools` so it can call
    /// `*_inner` methods that take `&self`. `label` is used in the panic
    /// error message.
    pub(super) async fn run_blocking<F, T>(&self, label: &'static str, f: F) -> Result<T>
    where
        F: FnOnce(Tools) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let tools = self.clone();
        tokio::task::spawn_blocking(move || f(tools))
            .await
            .map_err(|e| anyhow::anyhow!("{label} panic: {e}"))?
    }

    pub async fn execute(&self, mut call: ToolCall) -> Result<ToolResult> {
        call.args = normalize_tool_args(&call.tool, call.args);
        debug!(
            "Executing tool: {} args={}",
            call.tool,
            summarize_tool_args(&call.tool, &call.args)
        );
        match builtin::lookup(&call.tool) {
            Some(tool) => tool.execute(self, call).await,
            None => anyhow::bail!("unknown tool: {}", call.tool),
        }
    }

    /// AskUser is async-native — it uses tokio oneshot + timeout + Mutex
    /// internally and was previously bridged through `block_on_async`.
    pub(super) async fn ask_user(&self, args: serde_json::Value) -> Result<ToolResult> {
        let args: AskUserArgs = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid args for AskUser: {}", e))?;

        // Validate question count.
        if args.questions.is_empty() || args.questions.len() > 4 {
            anyhow::bail!("AskUser requires 1-4 questions, got {}", args.questions.len());
        }
        for (i, q) in args.questions.iter().enumerate() {
            if q.options.len() < 2 || q.options.len() > 6 {
                anyhow::bail!(
                    "AskUser question {} requires 2-6 options, got {}",
                    i, q.options.len()
                );
            }
        }

        // Sub-agents cannot use AskUser.
        if self.delegation_depth > 0 {
            return Ok(ToolResult::Success(
                self.prompt(crate::prompts::keys::ASKUSER_SUBAGENT_BLOCKED, &[]),
            ));
        }

        let bridge = match &self.ask_user_bridge {
            Some(b) => Arc::clone(b),
            None => {
                return Ok(ToolResult::Success(
                    self.prompt(crate::prompts::keys::ASKUSER_CLI_BLOCKED, &[]),
                ));
            }
        };

        let question_id = uuid::Uuid::new_v4().to_string();
        let agent_id = self.agent_id.clone().unwrap_or_default();
        let questions_clone = args.questions.clone();

        // Emit event to push the question to the UI.
        let _ = bridge.events_tx.send(crate::server::ServerEvent::AskUser {
            agent_id: agent_id.clone(),
            question_id: question_id.clone(),
            questions: args.questions,
            session_id: bridge.session_id.clone(),
        });

        // Register a oneshot channel for the response endpoint to deliver into.
        let (tx, rx) = tokio::sync::oneshot::channel();
        bridge.pending.lock().await.insert(
            question_id.clone(),
            PendingAskUser {
                agent_id,
                questions: questions_clone,
                sender: tx,
                session_id: bridge.session_id.clone(),
            },
        );

        // Wait up to 5 minutes for a response.
        let response = tokio::time::timeout(Duration::from_secs(300), rx).await;

        // Cleanup: remove from pending map regardless of outcome.
        bridge.pending.lock().await.remove(&question_id);

        match response {
            Ok(Ok(answers)) => Ok(ToolResult::AskUserResponse { answers }),
            Ok(Err(_)) => Ok(ToolResult::Success(
                self.prompt(crate::prompts::keys::ASKUSER_CANCELLED, &[]),
            )),
            Err(_) => Ok(ToolResult::Success(
                self.prompt(crate::prompts::keys::ASKUSER_TIMEOUT, &[]),
            )),
        }
    }
}

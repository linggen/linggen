mod background;
mod input;
mod message_filter;
mod rendering;
mod sse_handler;
mod utils;

use crate::server::UiSseMessage;
use crate::tui_client::TuiClient;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;
use tokio::sync::mpsc;

use super::display::*;

// Compact box-drawing logo (Calvin S figlet style)
const LOGO_1: &str = "╻  ╻┏┓╻┏━╸┏━╸┏━╸┏┓╻";
const LOGO_2: &str = "┃  ┃┃┗┫┃╺┓┃╺┓┣╸ ┃┗┫";
const LOGO_3: &str = "┗━╸╹╹ ╹┗━┛┗━┛┗━╸╹ ╹";

/// SSE connection status for the TUI.
pub enum ConnectionStatus {
    Connected,
    Disconnected,
}

/// In-progress tool group accumulator.
pub struct ActiveToolGroup {
    pub agent_id: String,
    pub steps: Vec<ToolStep>,
}

pub struct App {
    pub client: Arc<TuiClient>,
    pub sse_rx: mpsc::UnboundedReceiver<UiSseMessage>,
    pub input: String,
    pub banner: Vec<Line<'static>>,
    pub blocks: Vec<DisplayBlock>,
    pub project_root: String,
    pub agent_id: String,
    pub session_id: Option<String>,
    /// Shared slot for spawned tasks to write back an auto-created session_id.
    pub session_id_slot: Arc<StdMutex<Option<String>>>,
    // Status bar
    pub status_state: String,
    pub status_tool: Option<String>,
    pub status_agent: String,
    // Token streaming
    pub streaming_buffer: String,
    pub is_streaming: bool,
    /// True while model is in thinking phase (for animated indicator).
    pub is_thinking_phase: bool,
    /// App start time for animation timing.
    pub app_start: Instant,
    // Tool step tracking
    pub active_tool_group: Option<ActiveToolGroup>,
    // Subagent tracking
    pub active_subagents: HashMap<String, SubagentEntry>,
    /// Maps subagent_id → parent_agent_id for routing subagent activity.
    pub subagent_parent_map: HashMap<String, String>,
    // Dedup own messages echoed back via SSE
    pub pending_user_messages: VecDeque<String>,
    // Verbose/compact display mode
    pub verbose_mode: bool,
    /// Scroll offset from the bottom. 0 = follow tail, >0 = scrolled up.
    pub scroll_offset: usize,
    // Metrics tracking per agent
    pub last_context_tokens: HashMap<String, usize>,
    pub last_token_limit: HashMap<String, usize>,
    pub run_start_ts: HashMap<String, Instant>,
    // Interactive prompt (e.g., plan approval)
    pub prompt: Option<InteractivePrompt>,
    /// Base64-encoded images pending to be sent with the next message.
    pub pending_images: Vec<String>,
    /// Question ID for a pending AskUser/permission prompt.
    pub pending_ask_user_id: Option<String>,
    /// Autocomplete suggestions currently showing.
    pub autocomplete: Vec<AutocompleteItem>,
    /// Selected index in autocomplete list.
    pub autocomplete_selected: usize,
    /// Cached skills (name, description) fetched from server.
    pub cached_skills: Vec<(String, String)>,
    /// Cached agents (name, description) fetched from server.
    pub cached_agents: Vec<(String, String)>,
    /// Cached models (id, "provider: model_name") fetched from server.
    pub cached_models: Vec<(String, String)>,
    /// Shared slot for background task to deliver fetched skills.
    pub skills_slot: Arc<StdMutex<Option<Vec<(String, String)>>>>,
    /// Shared slot for background task to deliver fetched agents.
    pub agents_slot: Arc<StdMutex<Option<Vec<(String, String)>>>>,
    /// Shared slot for background task to deliver fetched models.
    pub models_slot: Arc<StdMutex<Option<Vec<(String, String)>>>>,
    /// SSE connection status (for reconnection indicator).
    pub connection_status: ConnectionStatus,
    /// Last seen SSE sequence number for dedup. Reset on reconnection.
    pub last_seq: u64,
    /// Text of the last agent message pushed (for deduplication across
    /// token-stream finalize, text_segment, and message events).
    pub last_agent_text: Option<String>,
}

/// An autocomplete suggestion item.
pub struct AutocompleteItem {
    pub label: String,
    pub description: String,
}

/// An interactive prompt shown inline for user selection.
/// The real input box stays active — typing goes there as free-form input.
pub struct InteractivePrompt {
    pub options: Vec<String>,
    pub selected: usize,
}

impl App {
    pub fn new(
        client: Arc<TuiClient>,
        sse_rx: mpsc::UnboundedReceiver<UiSseMessage>,
        project_root: String,
        port: u16,
    ) -> Self {
        let version = env!("CARGO_PKG_VERSION");
        let web_url = format!("http://localhost:{port}");

        let logo = Style::default().fg(Color::Cyan);
        let dim = Style::default().fg(Color::DarkGray);
        let label = Style::default().fg(Color::DarkGray);
        let val = Style::default().fg(Color::White);
        let accent = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        let sep = Style::default().fg(Color::DarkGray);

        let banner = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("  ", dim),
                Span::styled(LOGO_1, logo),
            ]),
            Line::from(vec![
                Span::styled("  ", dim),
                Span::styled(LOGO_2, logo),
                Span::styled(format!("   v{version}"), dim),
            ]),
            Line::from(vec![
                Span::styled("  ", dim),
                Span::styled(LOGO_3, logo),
                Span::styled("   AI Coding Agent", accent),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Web UI     ", label),
                Span::styled(web_url, val),
            ]),
            Line::from(vec![
                Span::styled("  Workspace  ", label),
                Span::styled(project_root.clone(), val),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "  /help  /agent <name>  /model <id>  @agent msg  /quit",
                dim,
            )]),
            Line::from(Span::styled(
                "  ──────────────────────────────────────────",
                sep,
            )),
            Line::from(""),
        ];

        Self {
            client,
            sse_rx,
            input: String::new(),
            banner,
            blocks: Vec::new(),
            project_root,
            agent_id: "ling".to_string(),
            session_id: None,
            session_id_slot: Arc::new(StdMutex::new(None)),
            status_state: "idle".to_string(),
            status_tool: None,
            status_agent: "ling".to_string(),
            streaming_buffer: String::new(),
            is_streaming: false,
            is_thinking_phase: false,
            app_start: Instant::now(),
            active_tool_group: None,
            active_subagents: HashMap::new(),
            subagent_parent_map: HashMap::new(),
            pending_user_messages: VecDeque::new(),
            verbose_mode: false,
            scroll_offset: 0,
            last_context_tokens: HashMap::new(),
            last_token_limit: HashMap::new(),
            run_start_ts: HashMap::new(),
            prompt: None,
            pending_images: Vec::new(),
            pending_ask_user_id: None,
            autocomplete: Vec::new(),
            autocomplete_selected: 0,
            cached_skills: Vec::new(),
            cached_agents: Vec::new(),
            cached_models: Vec::new(),
            skills_slot: Arc::new(StdMutex::new(None)),
            agents_slot: Arc::new(StdMutex::new(None)),
            models_slot: Arc::new(StdMutex::new(None)),
            connection_status: ConnectionStatus::Connected,
            last_seq: 0,
            last_agent_text: None,
        }
    }

    pub(super) fn push_user(&mut self, text: &str) {
        self.push_user_with_images(text, 0);
    }

    pub(super) fn push_user_with_images(&mut self, text: &str, image_count: usize) {
        self.blocks.push(DisplayBlock::UserMessage {
            text: text.to_string(),
            image_count,
        });
        // Clear dedup state so a new agent response can be tracked fresh.
        self.last_agent_text = None;
    }

    pub(super) fn push_agent(&mut self, agent: &str, text: &str) {
        self.blocks.push(DisplayBlock::AgentMessage {
            agent_id: agent.to_string(),
            text: text.to_string(),
        });
        self.last_agent_text = Some(text.to_string());
    }

    /// Returns true if `text` duplicates the last pushed agent message.
    pub(super) fn is_duplicate_agent_text(&self, text: &str) -> bool {
        self.last_agent_text.as_deref() == Some(text)
    }

    pub(super) fn context_usage_pct(&self) -> usize {
        let tokens = self
            .last_context_tokens
            .get(&self.agent_id)
            .copied()
            .unwrap_or(0);
        let limit = self
            .last_token_limit
            .get(&self.agent_id)
            .copied()
            .unwrap_or(200_000);
        if limit == 0 {
            return 0;
        }
        (tokens * 100) / limit
    }

    pub(super) fn push_system(&mut self, text: &str) {
        self.blocks.push(DisplayBlock::SystemMessage {
            text: text.to_string(),
        });
    }

    pub(super) fn finalize_streaming(&mut self) {
        if self.is_streaming && !self.streaming_buffer.is_empty() {
            let text = std::mem::take(&mut self.streaming_buffer);
            let agent = self.status_agent.clone();
            self.push_agent(&agent, &text);
        }
        self.streaming_buffer.clear();
        self.is_streaming = false;
    }

    pub(super) fn discard_streaming(&mut self) {
        self.streaming_buffer.clear();
        self.is_streaming = false;
    }

    /// Finalize the active tool group: push it as a ToolGroup display block.
    pub(super) fn finalize_tool_group(&mut self) {
        if let Some(group) = self.active_tool_group.take() {
            if !group.steps.is_empty() {
                let estimated_tokens = self.last_context_tokens.get(&group.agent_id).copied();
                let duration_secs = self
                    .run_start_ts
                    .get(&group.agent_id)
                    .map(|t| t.elapsed().as_secs());
                self.blocks.push(DisplayBlock::ToolGroup {
                    agent_id: group.agent_id,
                    steps: group.steps,
                    collapsed: !self.verbose_mode,
                    estimated_tokens,
                    duration_secs,
                });
            }
        }
    }

    /// Finalize active subagents into a SubagentGroup display block.
    pub(super) fn finalize_subagent_group(&mut self) {
        if !self.active_subagents.is_empty() {
            let entries: Vec<SubagentEntry> =
                self.active_subagents.drain().map(|(_, e)| e).collect();
            self.blocks.push(DisplayBlock::SubagentGroup { entries, collapsed: !self.verbose_mode });
        }
    }
}

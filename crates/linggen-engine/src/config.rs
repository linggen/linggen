use crate::util::default_true;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

mod explicit;
pub mod providers;
mod sign_in;
pub use explicit::Explicit;
pub use sign_in::{AuthMode, ProviderKind, SignIn};

/// Defaults for uses other than the legacy chat chain. Missing choices keep
/// the built-in order. Speaker stays in `pet.voice`, including for downgrade.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct Uses {
    #[serde(rename = "live", skip_serializing, deserialize_with = "ignore_preference")]
    _retired_live: (),
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speak: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draw: Option<String>,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub model_effort: std::collections::BTreeMap<String, String>,
    #[serde(rename = "chat_effort", skip_serializing)]
    legacy_chat_effort: Option<String>,
    // Retired preference: accept any old value without retaining it.
    #[serde(rename = "apps", skip_serializing, deserialize_with = "ignore_preference")]
    _retired_apps: (),
}

fn ignore_preference<'de, D: serde::Deserializer<'de>>(value: D) -> Result<(), D::Error> {
    serde::de::IgnoredAny::deserialize(value).map(|_| ())
}

impl Uses {
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Debug)]
pub struct DuplicateModelId;

impl std::fmt::Display for DuplicateModelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Duplicate model ID")
    }
}

impl std::error::Error for DuplicateModelId {}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(any(test, feature = "test-support"), derive(ts_rs::TS), ts(export))]
pub struct Config {
    #[serde(default)]
    pub models: Vec<ModelConfig>,
    pub server: ServerConfig,
    pub agent: AgentConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub agents: Vec<AgentSpecRef>,
    #[serde(default)]
    pub routing: RoutingConfig,
    #[serde(default, skip_serializing_if = "Uses::is_empty")]
    #[cfg_attr(any(test, feature = "test-support"), ts(skip))]
    pub uses: Uses,
    /// Default working folder for new sessions. Defaults to `~` if not set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_path: Option<String>,
    /// The language Linggen writes in for the person — a BCP-47 tag ("en",
    /// "zh-Hans"); unset = Auto (the Mac's, then English). Resolved by
    /// [`crate::language::effective`]. Named as Claude Code names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Yinyue's settings — see [`PetConfig`]. Surfaced in Settings → Agents
    /// beside Ling's model (Ling's is `routing.default_models[0]`). The key
    /// stays `[pet]` so existing files and paired phones keep working.
    #[serde(default)]
    pub pet: PetConfig,
    /// MCP servers this engine connects to as a *client* — the same shape
    /// every other host writes, so an entry copies across unchanged. See
    /// `doc/mcp-client-spec.md`.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub mcp_servers: std::collections::BTreeMap<String, crate::mcp_client::McpServerConfig>,
    /// The private apps the owner opened to agents outside Linggen (Claude
    /// Code, Codex) at the MCP door's `app` tool — Settings → Tools & MCP.
    /// Absent: none. Changed only by `PUT /api/mcp/outside-apps` at this
    /// Mac; a saved config keeps what it holds (`api::config`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outside_apps: Vec<String>,
    /// Which endpoints have had their starter families ticked, once
    /// (`families::Families::start_endpoint`). Absent: none. Written only
    /// by `families::binding`; a saved config keeps what it holds.
    #[serde(default, skip_serializing_if = "crate::provider::models::families::FamilyStarts::is_empty")]
    #[cfg_attr(any(test, feature = "test-support"), ts(skip))]
    pub family_starts: crate::provider::models::families::FamilyStarts,
    /// What agents outside Linggen (Claude Code, Codex, …) get as memory
    /// from Linggen — `[memory]` on disk. Hand-edited; a saved settings page
    /// keeps what it holds (`api::config`). Written only for what is set.
    #[serde(default, skip_serializing_if = "MemoryConfig::is_unset")]
    #[cfg_attr(any(test, feature = "test-support"), ts(skip))]
    pub memory: MemoryConfig,
    /// Which edition of the curated model list is offered (`[model_catalog]`):
    /// only what is offered for a new setup, never what a person already has.
    /// Absent: Auto. Written only by the owner's route (`PUT /api/providers/edition`);
    /// a saved config keeps what it holds.
    #[serde(default, skip_serializing_if = "crate::provider::models::ModelCatalog::is_unset")]
    #[cfg_attr(any(test, feature = "test-support"), ts(skip))]
    pub model_catalog: crate::provider::models::ModelCatalog,
    /// Monthly spending caps in USD, by provider id (`providers_view::provider_id`).
    /// Absent: none. Written only by `AgentManager::set_spend_cap`; a saved
    /// config keeps what it holds (`api::config`).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    #[cfg_attr(any(test, feature = "test-support"), ts(skip))]
    pub spend_caps: std::collections::BTreeMap<String, f64>,
    /// What was wrong in the file and was set right on load rather than
    /// refusing the whole config (a bad model `price` is dropped, that
    /// model alone). Runtime-only — never read from or written to the file —
    /// so the log and `ling status` can say it.
    #[serde(skip)]
    pub problems: Vec<String>,
}

/// How Linggen serves memory to agents outside it. The one place that
/// decides: the MCP door's server text, the plugin's hooks (they ask the
/// door, `memory_agents op=mode`) and the index it forwards all read it here.
/// Linggen's own agents (Ling, Yinyue, Linggen's sessions) are the same in
/// both.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutsideAgents {
    /// The default: the plugin has the host write into Linggen's store and
    /// recalls from it every turn.
    #[default]
    V1,
    /// Opt-in (`outside_agents = "v2"`): each agent keeps its own memory;
    /// Linggen reads it (never writes it) and forwards the other agents'
    /// index at session start.
    V2,
}

impl OutsideAgents {
    /// The word the door and the hooks use for it.
    pub fn word(self) -> &'static str {
        match self {
            OutsideAgents::V1 => "v1",
            OutsideAgents::V2 => "v2",
        }
    }
}

/// `[memory]` on disk. Each switch is [`Explicit`]: absent follows the
/// default of the running version; a written one — `"v1"` too — is the
/// person's and is always written back (`explicit.rs`). Read through the
/// accessors; the fields are private so nothing reads the raw choice.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    /// `outside_agents = "v1" | "v2"`; absent: v1 (v2 is opt-in).
    #[serde(skip_serializing_if = "Explicit::is_unset")]
    outside_agents: Explicit<OutsideAgents>,
    /// `linggen = "v1" | "v2"`: which memory Linggen's own sessions (Ling,
    /// Yinyue, apps) have; absent: v1 (v2 is opt-in). Read into `EngineConfig` and nowhere
    /// else (`own_memory::Mode`).
    #[serde(skip_serializing_if = "Explicit::is_unset")]
    linggen: Explicit<crate::own_memory::Mode>,
}

impl MemoryConfig {
    /// How outside agents get memory now.
    pub fn outside_agents(&self) -> OutsideAgents {
        self.outside_agents.get()
    }

    /// Which memory Linggen's own sessions have now.
    pub fn linggen(&self) -> crate::own_memory::Mode {
        self.linggen.get()
    }

    /// This section with `outside_agents` chosen.
    pub fn with_outside_agents(mut self, choice: OutsideAgents) -> Self {
        self.outside_agents = Explicit::set(choice);
        self
    }

    /// This section with `linggen` chosen.
    pub fn with_linggen(mut self, choice: crate::own_memory::Mode) -> Self {
        self.linggen = Explicit::set(choice);
        self
    }

    /// Whether either switch is v2: the person's memory is these
    /// notes, so the v1 store is moved into them once (`own_memory::migrate`).
    /// Both v1 — written so, or absent — is the person staying where they are.
    pub fn wants_v2(&self) -> bool {
        self.linggen() == crate::own_memory::Mode::V2 || self.outside_agents() == OutsideAgents::V2
    }

    /// Nothing chosen: no `[memory]` on disk.
    fn is_unset(&self) -> bool {
        self.outside_agents.is_unset() && self.linggen.is_unset()
    }
}

/// Settings for Yinyue, the companion agent (`[pet]` on disk). The recall fields
/// are intentionally separate from the global `agent.memory_*` knobs: a
/// companion does many small turns, so her recall stays tighter than a coding
/// session's.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(any(test, feature = "test-support"), derive(ts_rs::TS), ts(export))]
pub struct PetConfig {
    /// Master switch. When false the pet doesn't render and her event-reactive
    /// watch loop stays silent.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Her agent id ("yinyue"). Not shown in Settings — there is one Yinyue —
    /// but kept: `/api/pair/me` `agent_models` keys her model by it.
    #[serde(default = "default_pet")]
    pub pet: String,
    /// Show the speech-bubble text alongside her spoken audio.
    #[serde(default = "default_true")]
    pub show_text: bool,
    /// Memories injected per pet turn — kept tight for a snappy companion.
    #[serde(default = "default_pet_recall_count")]
    pub recall_count: usize,
    /// Per-row cosine score floor for her auto-recall.
    #[serde(default = "default_pet_recall_min_score")]
    pub recall_min_score: f32,
    /// LLM model for the companion. "auto" = the metered Linggen Cloud model for
    /// signed-in (paid/free) users, or the engine default for BYOK users; or a
    /// specific model id to pin. Applied per turn.
    #[serde(default = "default_pet_model")]
    pub model: String,
    /// Speaker for her synthesized voice (a Qwen3-TTS speaker name). Applied
    /// per clip; unknown names fall back to the provider default.
    #[serde(default = "default_pet_voice")]
    pub voice: String,
    /// Her voice is off on this Mac: she still writes every line — the
    /// bubble, her chat — and no audio is made or played. Set by `/mute` and
    /// `/unmute`, the `Voice` tool, or Settings; kept until turned back on.
    /// This Mac only: a phone keeps its own.
    #[serde(default)]
    pub muted: bool,
    /// The most lines she says unprompted in any rolling hour — a glance, an
    /// app moment, a woken note; a reply is never counted. 0: none at all.
    #[serde(default = "default_unprompted_per_hour")]
    pub unprompted_per_hour: u32,
    /// Local hours in which she says nothing unprompted, `HH:MM-HH:MM` (it
    /// may cross midnight). Empty: none.
    #[serde(default = "default_quiet_hours")]
    pub quiet_hours: String,
}

/// The unprompted lines an hour may hold unless the settings say otherwise.
pub const DEFAULT_UNPROMPTED_PER_HOUR: u32 = 6;
/// Her quiet hours unless the settings say otherwise.
pub const DEFAULT_QUIET_HOURS: &str = "23:00-07:00";

fn default_unprompted_per_hour() -> u32 {
    DEFAULT_UNPROMPTED_PER_HOUR
}
fn default_quiet_hours() -> String {
    DEFAULT_QUIET_HOURS.to_string()
}

fn default_pet_voice() -> String {
    "vivian".into()
}

impl Default for PetConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            pet: default_pet(),
            show_text: true,
            recall_count: default_pet_recall_count(),
            recall_min_score: default_pet_recall_min_score(),
            model: default_pet_model(),
            voice: default_pet_voice(),
            muted: false,
            unprompted_per_hour: default_unprompted_per_hour(),
            quiet_hours: default_quiet_hours(),
        }
    }
}

fn default_pet() -> String {
    "yinyue".to_string()
}
fn default_pet_model() -> String {
    "auto".to_string()
}
fn default_pet_recall_count() -> usize {
    1
}
fn default_pet_recall_min_score() -> f32 {
    0.8
}

/// A model's price in USD per million tokens (`[[models]] price = { … }`).
/// `cache_read` and `cache_write` default to `input`: a provider that does
/// not discount or bill its cache costs the plain input price. The price is
/// flat — it holds for every prompt size.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[cfg_attr(any(test, feature = "test-support"), derive(ts_rs::TS), ts(export))]
pub struct ModelPrice {
    pub input: f64,
    pub output: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(any(test, feature = "test-support"), derive(ts_rs::TS), ts(export))]
pub struct ModelConfig {
    pub id: String,
    pub provider: String, // "ollama" | "openai"
    pub url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub keep_alive: Option<String>,
    /// Manual context window override (tokens). Used when the provider API
    /// does not report context size (e.g. Ollama cloud/remote models).
    #[serde(default)]
    pub context_window: Option<usize>,
    /// Tags for model capabilities, e.g. ["vision"].
    #[serde(default)]
    pub tags: Vec<String>,
    /// Whether this model supports native function calling (OpenAI tools parameter).
    /// `None` = auto-detect based on provider. `Some(true)` = force enable.
    /// `Some(false)` = force disable (use legacy JSON action format).
    #[serde(default)]
    pub supports_tools: Option<bool>,
    /// Authentication mode: "api_key" (default) or "chatgpt_oauth".
    /// When "chatgpt_oauth", uses ChatGPT subscription OAuth tokens instead of API key.
    /// Read through [`ModelConfig::sign_in`] (`config/sign_in.rs`).
    #[serde(default)]
    #[cfg_attr(any(test, feature = "test-support"), ts(type = "string | null"))]
    pub auth_mode: Option<AuthMode>,
    /// Reasoning effort level: "low", "medium", "high".
    /// Translates to provider-specific parameters:
    /// - OpenAI/o-series/GPT-5: `reasoning_effort`
    /// - Gemini 2.5: `thinkingConfig.thinkingBudget`
    /// - Others: ignored (no-op)
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    /// The output limit (`max_tokens`) to send an Anthropic model, whatever
    /// the Models API says it can do. Clamped to the model's cap when that
    /// is known. Unset: the engine sends a working limit that fits the model
    /// (`provider/output_limit.rs`). Other providers ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    /// What this model's tokens cost, in USD per million tokens, when the
    /// built-in table (`provider/models/pricing.rs`) has no price for it or
    /// you pay another one (a gateway, a negotiated rate). Wins over the
    /// table. Unset and unlisted: the session shows tokens and no dollars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<ModelPrice>,
    /// Display name of the proxy room owner providing this model.
    /// Only set for proxy models (provider = "proxy").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provided_by: Option<String>,
    /// True for models injected by the engine (Linggen Cloud, ChatGPT)
    /// rather than configured by the user. Never persisted by the user —
    /// only set at runtime by inject_linggen_cloud/inject_chatgpt_builtin —
    /// so Settings can render them as read-only built-in cards instead of
    /// editable entries, regardless of what's in the user's config file.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_builtin: bool,
    /// Set on the one row a ticked family keeps (`provider/models/families`):
    /// it names the family the row follows or is pinned in, and the last move
    /// a person has yet to acknowledge. A released 2.6.1 binary reads the
    /// other fields and drops this one on save; the row then runs as an
    /// exact row, and nothing infers a family from its id or its model.
    /// Written only by `families::binding`; read through its accessors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "test-support"), ts(skip))]
    pub family_binding: Option<crate::provider::models::families::FamilyBinding>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(any(test, feature = "test-support"), derive(ts_rs::TS), ts(export))]
pub struct AgentSpecRef {
    pub id: String,
    pub spec_path: String,
    pub model: Option<String>,
}

/// The daemon's default listen port. Single source of truth — every engine
/// consumer of the default (CLI, config default, install/autostart scripts,
/// clients) resolves to this rather than a scattered literal. Override per
/// install via `[server].url` in linggen.toml or `--port`.
pub const DEFAULT_PORT: u16 = 9527;

/// Where this machine's daemon listens.
///
/// **This is the SERVE declaration** — where the engine on this machine binds.
/// It is not where a *client* goes looking: a second machine reaching this one
/// has its own client config, and reading a server's file to find a server on
/// another host is how those two facts get confused.
///
/// One field, `url`, because a listen address is one fact. It was `host` +
/// `port`, two fields that could only ever be used together, and every launcher
/// that wanted "the address" had to reassemble them — which is how three of them
/// ended up assembling their own default instead.
///
/// `host` and `port` still parse so an existing install keeps working; when
/// present without `url` they are used, logged once as superseded, and replaced
/// by `url` the next time the config is written.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(any(test, feature = "test-support"), derive(ts_rs::TS), ts(export))]
pub struct ServerConfig {
    /// `host:port` — e.g. `127.0.0.1:9527`, or `0.0.0.0:9527` to accept the LAN.
    /// A `http://` prefix is accepted and ignored, since that is how the same
    /// address is written everywhere it is *consumed*.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// Superseded by `url`. Kept readable, never written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Superseded by `url`. Kept readable, never written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,

    /// Set at load when a legacy pair was folded into `url`. Runtime-only —
    /// never read from or written to the file — so the startup banner can say
    /// so at a point where the log actually goes somewhere.
    #[serde(skip)]
    pub migrated_legacy_addr: bool,
}

impl ServerConfig {
    /// The bind address this config asks for, as `(host, port)`.
    ///
    /// `url` wins; then the legacy pair; then the defaults. Resolved in one
    /// place so no caller has to know the precedence — the reason the two-field
    /// shape drifted is that each caller reassembled it privately.
    pub fn addr(&self) -> (String, u16) {
        if let Some(raw) = self.url.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            if let Some(parsed) = parse_addr(raw) {
                return parsed;
            }
            tracing::warn!(
                "[server].url = {raw:?} is not host:port — falling back to {}:{DEFAULT_PORT}",
                default_server_host()
            );
        }
        (
            self.host.clone().unwrap_or_else(default_server_host),
            self.port.unwrap_or(DEFAULT_PORT),
        )
    }

    pub fn host(&self) -> String {
        self.addr().0
    }

    pub fn port(&self) -> u16 {
        self.addr().1
    }

    /// Fold a legacy `host` + `port` pair into `url`, so the next write emits
    /// one field. Returns whether it did, because config load runs BEFORE the
    /// tracing subscriber exists — a message logged here goes nowhere, and a
    /// config that changes shape under the user in silence is the hidden state
    /// this whole change is against. The caller reports it where the user looks.
    pub fn migrate_legacy_fields(&mut self) -> bool {
        if self.url.is_some() || (self.host.is_none() && self.port.is_none()) {
            return false;
        }
        let (host, port) = self.addr();
        self.url = Some(format!("{host}:{port}"));
        self.host = None;
        self.port = None;
        true
    }
}

/// `host:port`, with an optional scheme that is accepted and dropped.
fn parse_addr(raw: &str) -> Option<(String, u16)> {
    let rest = raw.split_once("://").map(|(_, r)| r).unwrap_or(raw);
    let rest = rest.trim_end_matches('/');
    let (host, port) = rest.rsplit_once(':')?;
    let host = host.trim();
    if host.is_empty() {
        return None;
    }
    Some((host.to_string(), port.trim().parse().ok()?))
}

fn default_server_host() -> String {
    "127.0.0.1".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(any(test, feature = "test-support"), derive(ts_rs::TS), ts(export))]
pub struct AgentConfig {
    /// Most steps (a model reply and the tool calls in it) one turn may take.
    /// A coding session has [`coding_max_iters`](Self::coding_max_iters).
    pub max_iters: usize,
    /// The same limit for a coding session (`work: code`): a long job is
    /// many steps, and a limit tuned for chat ends it half done. Defaults to
    /// the most the config allows ([`MAX_ITERS_CEILING`]).
    #[serde(default = "default_coding_max_iters")]
    pub coding_max_iters: usize,
    #[serde(default)]
    pub write_safety_mode: WriteSafetyMode,
    /// Default permission for new sessions (chat/read/edit/admin). Defaults to read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_permission_mode: Option<crate::permission_mode::PermissionMode>,
    #[serde(default)]
    pub prompt_loop_breaker: Option<String>,
    #[serde(default = "default_max_delegation_depth")]
    pub max_delegation_depth: usize,
    /// Global auto-compaction trigger as a fraction of context_window_tokens.
    /// 0.10–0.99. None = use hardcoded engine default (0.95). Per-session
    /// override (set via POST /api/chat/compact_config) takes precedence.
    /// See `engine/context.rs::context_soft_token_limit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_threshold: Option<f32>,
    /// Episodic-memory retention in days. Episodic rows past this age are
    /// terminally decided by the user-triggered `dream` mission, then swept
    /// by the evict backstop. Default 7. See `memory-spec.md` §2.
    #[serde(default = "default_episodic_ttl_days")]
    pub episodic_ttl_days: u64,

    /// Per-row relevance floor for per-turn auto-recall, as an OVERRIDE of
    /// the ling-mem daemon's store-wide `recall_min_score`. The daemon gates
    /// its HYBRID score (cosine + IDF keyword boost), not the raw cosine.
    /// Default 0.7. `None` (explicit, not the default) = defer to the
    /// daemon's configured floor (the engine omits `min_score`). Range 0.0–1.0.
    #[serde(default = "default_memory_inject_min_score")]
    pub memory_inject_min_score: Option<f32>,

    /// How many recalled memories are injected per turn — the top-K cap
    /// applied after the project-scope filter. Default 3.
    #[serde(default = "default_memory_recall_count")]
    pub memory_recall_count: usize,

    /// Ask the model for follow-up buttons on the turns of a chat that shows
    /// them (the Mac chat). Off: no asking; a skill's starters and a page's
    /// buttons still show. See `chat-spec.md` § Suggestions.
    #[serde(default = "default_true")]
    pub suggest_followups: bool,
    /// Linggen's daily session (`doc/one-ling-spec.md` § Linggen's daily session): app
    /// chats, Yinyue's unprompted moments and the phone's hand-offs are the daily session, Ling and Yinyue both seated, each turn in the place
    /// of the page it came from. Off: a session per app, as before.
    /// Absent means no choice was made; either explicit choice is saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "test-support"), ts(optional))]
    pub daily_session: Option<bool>,
}

/// Linggen's daily session is on unless a person saved a choice (decided
/// by the builder 2026-11-18).
pub const DAILY_SESSION_DEFAULT: bool = true;

impl AgentConfig {
    pub fn daily_session_enabled(&self) -> bool {
        self.daily_session_with_default(DAILY_SESSION_DEFAULT)
    }

    fn daily_session_with_default(&self, default: bool) -> bool {
        self.daily_session.unwrap_or(default)
    }
}

fn default_episodic_ttl_days() -> u64 {
    7
}

fn default_memory_inject_min_score() -> Option<f32> {
    Some(0.7)
}

fn default_memory_recall_count() -> usize {
    3
}

/// The most steps any limit may be set to.
pub const MAX_ITERS_CEILING: usize = 1000;

fn default_coding_max_iters() -> usize {
    MAX_ITERS_CEILING
}

fn default_max_delegation_depth() -> usize {
    2
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
#[cfg_attr(any(test, feature = "test-support"), derive(ts_rs::TS), ts(export))]
pub enum WriteSafetyMode {
    Strict,
    #[default]
    Warn,
    Off,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[cfg_attr(any(test, feature = "test-support"), derive(ts_rs::TS), ts(export))]
pub struct LoggingConfig {
    pub level: Option<String>,
    pub directory: Option<String>,
    pub retention_days: Option<u64>,
}

/// A key missing from the file takes its value from [`RoutingConfig::default`]
/// (the container-level `serde(default)`), so each default is written once —
/// no field here may carry its own `#[serde(default…)]`.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
#[cfg_attr(any(test, feature = "test-support"), derive(ts_rs::TS), ts(export))]
pub struct RoutingConfig {
    // Retired keys `default_policy` / `policies` (named routing policies that
    // never ran) are ignored on load — serde skips unknown keys.
    /// Ordered list of model IDs selected as defaults by the user.
    /// The first model in the list is the primary default; others are fallbacks.
    pub default_models: Vec<String>,
    /// Retired with the Claude Agent SDK session (2.6.0): the 2.5.1 default
    /// for Claude sessions. Read on load only, by
    /// [`Config::migrate_retired_claude_default`], never written.
    #[serde(skip_serializing)]
    #[cfg_attr(any(test, feature = "test-support"), ts(skip))]
    claude_default_model: Option<String>,
    // The retired `auto_fallback` key is ignored, whatever its value.
    /// When true, automatically try the next model on transient errors
    /// (timeout, rate limit, 502/503, connection failures). Default: true.
    pub model_fallback: bool,
    /// Seconds a model may say nothing at all — no response headers, no
    /// first stream event — before the call is abandoned and retried (then
    /// moved down the chain). Unset: 30. `0` turns the guard off. A backend
    /// on this machine (localhost, loopback) is exempt.
    pub first_event_timeout_secs: Option<u64>,
}

/// The one place routing defaults are decided: a fresh config, a file
/// without `[routing]`, a `[routing]` missing a key, and a new engine
/// (`AgentEngine::new`) all read them here.
impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            default_models: Vec::new(),
            claude_default_model: None,
            model_fallback: true,
            first_event_timeout_secs: None,
        }
    }
}

impl Config {
    /// The files a config is looked for in, first found wins.
    pub fn candidate_paths() -> Vec<PathBuf> {
        let mut candidates = Vec::new();

        if let Ok(explicit) = std::env::var("LINGGEN_CONFIG") {
            candidates.push(PathBuf::from(explicit));
        }

        // ~/.linggen/config/
        let cfg_dir = crate::paths::config_dir();
        candidates.push(cfg_dir.join("linggen.runtime.toml"));
        candidates.push(cfg_dir.join("linggen.toml"));
        candidates
    }

    pub fn load_with_path() -> Result<(Self, Option<PathBuf>)> {
        for path in Self::candidate_paths() {
            if path.exists() {
                let content = fs::read_to_string(&path)?;
                let config = Config::from_toml(&content)
                    .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
                crate::provider::set_first_event_timeout(config.routing.first_event_timeout_secs);
                return Ok((config, Some(path)));
            }
        }

        Ok((Config::default(), None))
    }

    /// Keep the model fallback switch when a different settings page saves.
    pub fn keep_model_fallback(&mut self, current: &Self) {
        self.set_model_fallback(current.routing.model_fallback);
    }

    pub fn set_model_fallback(&mut self, enabled: bool) {
        self.routing.model_fallback = enabled;
    }

    /// A config file's text, read and brought up to date.
    pub fn from_toml(content: &str) -> Result<Self> {
        let mut config: Config = toml::from_str(content)?;
        config.server.migrated_legacy_addr = config.server.migrate_legacy_fields();
        config.migrate_retired_chatgpt_builtins();
        config.migrate_retired_cloud_ids();
        config.migrate_retired_claude_default();
        if let (Some(model), Some(value)) = (config.routing.default_models.first(), config.uses.legacy_chat_effort.take()) {
            config.uses.model_effort.entry(model.clone()).or_insert(value);
        }
        config.drop_bad_prices();
        Ok(config)
    }

    /// What is wrong with a model's `price`, naming the model and the field.
    /// A price is real money: every number finite and not negative, and only
    /// on a model paid by the token — one that bills an account (a ChatGPT
    /// sign-in, Linggen Cloud, a room, a subscription) would never use it.
    fn price_problem(model: &ModelConfig) -> Option<String> {
        let price = model.price.as_ref()?;
        let fields = [
            ("price.input", Some(price.input)),
            ("price.output", Some(price.output)),
            ("price.cache_read", price.cache_read),
            ("price.cache_write", price.cache_write),
        ];
        for (field, value) in fields {
            if let Some(n) = value.filter(|n| !n.is_finite() || *n < 0.0) {
                return Some(format!(
                    "Model '{}': {field} must be a finite number, not negative (got {n}).",
                    model.id
                ));
            }
        }
        (crate::provider::models::billing_of(model) == crate::provider::models::Billing::Account)
            .then(|| {
                format!(
                    "Model '{}' has a price, but it bills an account, not by the token: remove its price.",
                    model.id
                )
            })
    }

    /// Strict, for a config being written (`validate`): the first bad price
    /// refuses the save.
    fn check_prices(&self) -> Result<()> {
        match self.models.iter().find_map(Self::price_problem) {
            Some(why) => anyhow::bail!("{why}"),
            None => Ok(()),
        }
    }

    /// Lenient, for a config being read: one bad price must never cost the
    /// person the rest of their file. That model loses its price (it then
    /// shows tokens and no dollars) and the problem is kept in `problems`
    /// for the log and `ling status`.
    fn drop_bad_prices(&mut self) {
        for model in &mut self.models {
            if let Some(why) = Self::price_problem(model) {
                model.price = None;
                self.problems.push(format!("{why} Its price is ignored."));
            }
        }
    }

    /// Bring persisted references in line with the ChatGPT catalog in force.
    fn migrate_retired_chatgpt_builtins(&mut self) {
        self.follow_chatgpt_catalog(&crate::provider::models::chatgpt_catalog::current());
    }

    /// Follow the ChatGPT families in [catalog]: drop stale copies of an old
    /// built-in (same id + chatgpt_oauth — a custom API-key entry that
    /// happens to reuse the id is kept), and move routing defaults and the
    /// pet's pin from an older id of a family to the family's model now,
    /// when the older id no longer names any configured model. A family's
    /// model is only ever one that has answered, so a default never moves
    /// to a model that cannot. True when anything changed.
    pub fn follow_chatgpt_catalog(
        &mut self,
        catalog: &crate::provider::models::chatgpt_catalog::Snapshot,
    ) -> bool {
        let before = (
            self.models.len(),
            self.routing.default_models.clone(),
            self.pet.model.clone(),
        );
        self.models.retain(|m| {
            !(catalog.is_former(&m.id) && m.auth_mode == Some(AuthMode::ChatgptOauth))
        });

        let surviving: std::collections::HashSet<String> =
            self.models.iter().map(|m| m.id.clone()).collect();
        let successor = |id: &str| catalog.follow(id).filter(|_| !surviving.contains(id));
        for dm in &mut self.routing.default_models {
            if let Some(next) = successor(dm) {
                *dm = next;
            }
        }
        let mut seen = std::collections::HashSet::new();
        self.routing
            .default_models
            .retain(|id| seen.insert(id.clone()));
        if let Some(next) = successor(&self.pet.model) {
            self.pet.model = next;
        }
        before
            != (
                self.models.len(),
                self.routing.default_models.clone(),
                self.pet.model.clone(),
            )
    }

    /// The configured rows a chat model is picked from: every row but a
    /// live-voice one (`provider::kinds`). Any list of models built from the
    /// config's own rows starts here, never from `self.models` (a lock in
    /// `src/server/model_locks.rs` counts the readers of the raw rows).
    pub fn chat_models(&self) -> impl Iterator<Item = &ModelConfig> {
        use crate::provider::kinds::{kind_in, Kind};
        let facts = crate::provider::models::curated::current();
        self.models.iter().filter(move |m| kind_in(m, &facts) == Kind::Chat)
    }

    /// Whether `id` names a configured live-voice row.
    pub fn is_live_voice_row(&self, id: &str) -> bool {
        use crate::provider::kinds::{kind_of, Kind};
        self.models
            .iter()
            .any(|m| m.id == id && kind_of(m) == Kind::LiveVoice)
    }

    /// A live-voice model in a place that runs chat turns — the default
    /// chain, the companion's model, an agent's model — is refused at save.
    fn check_no_live_voice_model_in_chat_places(&self) -> Result<()> {
        let chain = self.routing.default_models.iter().map(|id| ("routing.default_models", id));
        let pet = std::iter::once(("pet.model", &self.pet.model));
        let agents = self
            .agents
            .iter()
            .filter_map(|a| a.model.as_ref().map(|id| ("agents[].model", id)));
        for (place, id) in chain.chain(pet).chain(agents) {
            if self.is_live_voice_row(id) {
                anyhow::bail!(
                    "'{id}' is a live-voice model; it cannot be used for chat ({place}). Pick a chat model."
                );
            }
        }
        Ok(())
    }

    /// An id no model row has: the prefix and ten random characters. The one
    /// place a row's id is made for a person (a ticked family, a typed model).
    pub fn fresh_model_id(&self, prefix: &str) -> String {
        loop {
            let id = format!("{prefix}-{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
            if !self.models.iter().any(|row| row.id == id) {
                return id;
            }
        }
    }

    /// Replace the default; only explicitly selected fallbacks stay in the chain.
    pub fn make_default(&mut self, id: &str) {
        let chain = std::mem::take(&mut self.routing.default_models);
        self.routing.default_models = std::iter::once(id.to_owned())
            .chain(chain.into_iter().skip(1).filter(|model| model != id))
            .collect();
    }

    /// Upgrade from 2.5.1, kept for safe operation: the retired Claude
    /// sessions' default becomes the default model when the config named no
    /// other, so a person who ran only Claude keeps working on it — natively —
    /// with no step of theirs. Test:
    /// `a_claude_session_default_is_read_as_the_native_default`.
    fn migrate_retired_claude_default(&mut self) {
        let Some(model) = self.routing.claude_default_model.take() else {
            return;
        };
        if self.routing.default_models.is_empty() && !model.is_empty() {
            self.routing.default_models.push(model);
        }
    }

    /// Re-point persisted references to a retired Linggen Cloud id (routing
    /// defaults, the pet's pin) at the current one. A user-defined model that
    /// happens to reuse the retired id is theirs and is left alone.
    fn migrate_retired_cloud_ids(&mut self) {
        use crate::provider::models::{LINGGEN_CLOUD_MODEL_ID, LINGGEN_CLOUD_RETIRED_MODEL_IDS};
        let retired = |id: &str| {
            LINGGEN_CLOUD_RETIRED_MODEL_IDS.contains(&id) && !self.models.iter().any(|m| m.id == id)
        };
        let mut defaults = std::mem::take(&mut self.routing.default_models);
        for dm in &mut defaults {
            if retired(dm) {
                *dm = LINGGEN_CLOUD_MODEL_ID.to_string();
            }
        }
        let mut seen = std::collections::HashSet::new();
        defaults.retain(|id| seen.insert(id.clone()));
        self.routing.default_models = defaults;
        if retired(&self.pet.model) {
            self.pet.model = LINGGEN_CLOUD_MODEL_ID.to_string();
        }
    }

    /// What `ling status` says about the config files themselves: one that
    /// exists but cannot be loaded (the engine then runs on defaults), and
    /// copies of earlier unloadable files set aside beside them
    /// (`save_runtime`).
    pub fn health_lines(candidates: &[PathBuf]) -> Vec<String> {
        let mut lines = Vec::new();
        for path in candidates.iter().filter(|p| p.exists()) {
            if let Err(why) = loadable(path) {
                lines.push(format!(
                    "{} could not be read ({why}); the engine runs on defaults until it is fixed.",
                    path.display()
                ));
            }
            let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else { continue };
            let prefix = format!("{}{UNREADABLE_MARK}", name.to_string_lossy());
            let kept = std::fs::read_dir(dir).into_iter().flatten().flatten();
            for entry in kept.filter(|e| e.file_name().to_string_lossy().starts_with(&prefix)) {
                lines.push(format!(
                    "An earlier config that did not load was set aside and kept at {}.",
                    entry.path().display()
                ));
            }
        }
        lines
    }

    pub fn runtime_config_path(config_dir: Option<&Path>) -> PathBuf {
        if let Some(dir) = config_dir {
            return dir.join("linggen.runtime.toml");
        }
        crate::paths::config_dir().join("linggen.runtime.toml")
    }

    /// The one place a config is written. A file already there that cannot
    /// be loaded is the person's, not ours to replace with defaults: it is
    /// first set aside whole (`<name>.unreadable-<timestamp>`); if that, or
    /// reading it, fails, nothing is written.
    pub fn save_runtime(&self, config_dir: Option<&Path>) -> Result<PathBuf> {
        let path = Self::runtime_config_path(config_dir);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        set_aside_if_unloadable(&path)?;
        // Whole or not at all: a save beside this one, or a start after a
        // crash, never reads half a file (and so never sets a good one aside).
        crate::state_fs::devices::write_atomic(&path, toml::to_string_pretty(self)?.as_bytes())?;
        Ok(path)
    }

    pub fn validate(&self) -> Result<()> {
        // No `models.is_empty()` check: ModelManager always injects the
        // built-in models (Linggen Cloud, ChatGPT) at runtime regardless of
        // what's configured here, so an empty list is a valid "use only the
        // built-ins" config, not a broken one.
        self.check_prices()?;
        self.check_no_live_voice_model_in_chat_places()?;
        let mut seen_ids = std::collections::HashSet::new();
        for model in &self.models {
            if model.id.trim().is_empty() {
                anyhow::bail!("Model ID cannot be empty");
            }
            if !seen_ids.insert(&model.id) {
                return Err(DuplicateModelId.into());
            }
            if model.model.trim().is_empty() {
                anyhow::bail!(
                    "Model '{}' has an empty model name. Set the 'model' field to the actual model name (e.g. gemini-2.0-flash).",
                    model.id
                );
            }
            // Validate provider is known.
            let known = providers::known_names();
            if !known.contains(&model.provider_name()) {
                anyhow::bail!(
                    "Model '{}' has unknown provider '{}'. Known providers: {}",
                    model.id,
                    model.provider,
                    known.join(", ")
                );
            }
            // Validate model URL scheme to prevent SSRF.
            let url_lower = model.url.trim().to_lowercase();
            if !url_lower.starts_with("http://") && !url_lower.starts_with("https://") {
                anyhow::bail!(
                    "Model '{}' URL must start with http:// or https://, got: {}",
                    model.id,
                    model.url
                );
            }
            // keep_alive is forwarded verbatim to Ollama, whose Go duration
            // parser rejects a bare number ("10") with an opaque 400 at chat
            // time — catch it here at save time instead.
            if let Some(ka) = &model.keep_alive {
                if !is_valid_go_duration(ka.trim()) {
                    anyhow::bail!(
                        "Model '{}' keep_alive '{}' needs a time unit — e.g. '10m', '30s', '1h'.",
                        model.id,
                        ka
                    );
                }
            }
        }
        if self.server.port() == 0 {
            anyhow::bail!("Server port must be greater than 0");
        }
        if self.agent.max_iters == 0 {
            anyhow::bail!("Agent max_iters must be greater than 0");
        }
        if self.agent.max_iters > MAX_ITERS_CEILING {
            anyhow::bail!("Agent max_iters must not exceed {MAX_ITERS_CEILING}");
        }
        if self.agent.coding_max_iters == 0 {
            anyhow::bail!("Agent coding_max_iters must be greater than 0");
        }
        if self.agent.coding_max_iters > MAX_ITERS_CEILING {
            anyhow::bail!("Agent coding_max_iters must not exceed {MAX_ITERS_CEILING}");
        }
        // A 0-day TTL would evict episodic rows immediately on the next
        // dream pass, before the user has had a chance to inspect them.
        if self.agent.episodic_ttl_days == 0 {
            anyhow::bail!("Agent episodic_ttl_days must be greater than 0");
        }
        if let Some(s) = self.agent.memory_inject_min_score {
            if !(0.0..=1.0).contains(&s) || s.is_nan() {
                anyhow::bail!(
                    "Agent memory_inject_min_score must be between 0.0 and 1.0 (got {s})"
                );
            }
        }
        if !(1..=20).contains(&self.agent.memory_recall_count) {
            anyhow::bail!(
                "Agent memory_recall_count must be between 1 and 20 (got {})",
                self.agent.memory_recall_count
            );
        }
        // Warn (log) if default_models references non-existent model IDs.
        // Built-in models (injected at ModelManager build) are valid defaults.
        for dm in &self.routing.default_models {
            if dm == crate::provider::models::LINGGEN_CLOUD_MODEL_ID
                || crate::provider::models::chatgpt_catalog::is_builtin(dm)
            {
                continue;
            }
            if !seen_ids.contains(&dm) {
                tracing::warn!(
                    "routing.default_models references unknown model ID '{}'; it will be ignored",
                    dm
                );
            }
        }
        Ok(())
    }
}

const UNREADABLE_MARK: &str = ".unreadable-";

/// Whether the file at `path` loads as a config; the reason if not.
fn loadable(path: &Path) -> std::result::Result<(), String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    // The first line of the parser's report: where it stopped.
    Config::from_toml(&text)
        .map(|_| ())
        .map_err(|e| e.to_string().lines().next().unwrap_or_default().to_string())
}

/// A file that exists and cannot be loaded is moved aside before a config is
/// written in its place. A file that could not even be read is not touched
/// and nothing is written over it.
fn set_aside_if_unloadable(path: &Path) -> Result<()> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => anyhow::bail!("{} cannot be read ({e}); nothing was written over it", path.display()),
    };
    let loads = String::from_utf8(bytes).is_ok_and(|text| Config::from_toml(&text).is_ok());
    if loads {
        return Ok(());
    }
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!("{UNREADABLE_MARK}{}", chrono::Utc::now().timestamp()));
    let aside = path.with_file_name(name);
    fs::rename(path, &aside).map_err(|e| {
        anyhow::anyhow!(
            "{} cannot be loaded and could not be set aside ({e}); nothing was written over it",
            path.display()
        )
    })?;
    tracing::warn!(
        "[config] {} could not be loaded; kept as {} before the new config was written",
        path.display(),
        aside.display()
    );
    Ok(())
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // No hardcoded models — the built-in Linggen Cloud (deepseek-flash)
            // and ChatGPT (chatgpt_catalog) models are always injected by ModelManager
            // at runtime (see inject_linggen_cloud / inject_chatgpt_builtin in
            // provider/models.rs), so a fresh install needs nothing here.
            models: Vec::new(),
            server: ServerConfig {
                url: Some(format!("{}:{DEFAULT_PORT}", default_server_host())),
                port: None,
                host: None,
                migrated_legacy_addr: false,
            },
            agent: AgentConfig {
                max_iters: 200,
                coding_max_iters: default_coding_max_iters(),
                write_safety_mode: WriteSafetyMode::default(),
                default_permission_mode: None,
                prompt_loop_breaker: None,
                max_delegation_depth: default_max_delegation_depth(),
                compact_threshold: None,
                episodic_ttl_days: default_episodic_ttl_days(),
                memory_inject_min_score: default_memory_inject_min_score(),
                memory_recall_count: default_memory_recall_count(),
                suggest_followups: true,
                daily_session: None,
            },
            logging: LoggingConfig {
                level: None,
                directory: None,
                retention_days: None,
            },
            agents: Vec::new(),
            routing: RoutingConfig {
                // Fresh installs default to the built-in Linggen Cloud model —
                // works with zero auth setup, unlike the ChatGPT/chatgpt_oauth
                // entry above. Once the user stars a different model in
                // Settings, their explicit choice overwrites this.
                default_models: vec![crate::provider::models::LINGGEN_CLOUD_MODEL_ID.to_string()],
                ..Default::default()
            },
            uses: Uses::default(),
            home_path: None,
            language: None,
            pet: PetConfig::default(),
            mcp_servers: Default::default(),
            outside_apps: Vec::new(),
            family_starts: Default::default(),
            memory: MemoryConfig::default(),
            model_catalog: Default::default(),
            spend_caps: Default::default(),
            problems: Vec::new(),
        }
    }
}

/// Mirrors Go's `time.ParseDuration` grammar (what Ollama's `keep_alive`
/// expects): "0", or a possibly signed sequence of decimal numbers each
/// followed by a unit (ns/us/µs/ms/s/m/h), e.g. "10m", "90s", "1h30m".
fn is_valid_go_duration(s: &str) -> bool {
    let s = s.strip_prefix(['+', '-']).unwrap_or(s);
    if s == "0" {
        return true;
    }
    let mut rest = s;
    if rest.is_empty() {
        return false;
    }
    while !rest.is_empty() {
        let num_len = rest
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(rest.len());
        if num_len == 0 {
            return false;
        }
        rest = &rest[num_len..];
        // Two-char units before their one-char prefixes ("ms" before "m"/"s").
        let unit = ["ns", "us", "µs", "ms", "s", "m", "h"]
            .iter()
            .find(|u| rest.starts_with(**u));
        match unit {
            Some(u) => rest = &rest[u.len()..],
            None => return false,
        }
    }
    true
}

// The provider names a model may carry are `providers::known_names()`, a
// fixed list in `config/providers.rs`; what a page offers is the curated list.

#[cfg(test)]
mod tests {
    use super::*;

    /// `[memory]` is v1 unless the file says v2 (v2 is opt-in); a value the
    /// file says is kept, v1 too — only a never-set one is left out.
    #[test]
    fn memory_is_v1_unless_the_file_says_v2() {
        use crate::own_memory::Mode;
        let base = toml::to_string(&Config::default()).unwrap();
        let default: Config = toml::from_str(&base).unwrap();
        assert_eq!(default.memory.outside_agents(), OutsideAgents::V1);
        assert_eq!(default.memory.linggen(), Mode::V1);
        assert!(!default.memory.wants_v2());
        assert!(!toml::to_string(&default).unwrap().contains("[memory]"));
        // Written as v2, it is v2 and is written back so.
        let v2: Config = toml::from_str(&format!("{base}\n[memory]\nlinggen = \"v2\"\noutside_agents = \"v2\"\n")).unwrap();
        assert_eq!((v2.memory.linggen(), v2.memory.outside_agents()), (Mode::V2, OutsideAgents::V2));
        assert!(v2.memory.wants_v2());
        let text = toml::to_string(&v2).unwrap();
        assert!(text.contains("outside_agents = \"v2\"") && text.contains("linggen = \"v2\""), "{text}");
        // Written as v1, it stays v1.
        let v1: Config = toml::from_str(&format!("{base}\n[memory]\nlinggen = \"v1\"\noutside_agents = \"v1\"\n")).unwrap();
        assert!(!v1.memory.wants_v2());
        // One switch written as v2 is v2 for the move of the old rows.
        let half: Config = toml::from_str(&format!("{base}\n[memory]\nlinggen = \"v2\"\n")).unwrap();
        assert!(half.memory.wants_v2());
        assert_eq!(OutsideAgents::V2.word(), "v2");
    }

    /// The bug of 2026-11-25: a save left out a value for equalling the
    /// default, so when a later version moved the default the person's word
    /// moved with it. A choice is written even when it equals the default;
    /// what was never chosen stays absent. Through the one writer
    /// (`save_runtime`) and back through the one reader (`from_toml`).
    #[test]
    fn a_saved_config_keeps_what_the_person_chose_even_when_it_equals_the_default() {
        use crate::own_memory::Mode;
        use crate::provider::models::{EditionChoice, ModelCatalog};
        let dir = tempfile::tempdir().unwrap();
        let saved = |config: Config| {
            let path = config.save_runtime(Some(dir.path())).unwrap();
            let text = std::fs::read_to_string(path).unwrap();
            (Config::from_toml(&text).unwrap(), text)
        };
        let mut chosen = valid_config();
        chosen.memory = MemoryConfig::default().with_outside_agents(OutsideAgents::V1).with_linggen(Mode::V1);
        chosen.model_catalog = ModelCatalog::chosen(EditionChoice::Auto);
        let (back, text) = saved(chosen);
        assert!(text.contains("[memory]") && text.contains("outside_agents = \"v1\"") && text.contains("linggen = \"v1\""), "{text}");
        assert!(text.contains("[model_catalog]") && text.contains("edition = \"auto\""), "{text}");
        assert_eq!(back.memory, MemoryConfig::default().with_outside_agents(OutsideAgents::V1).with_linggen(Mode::V1));
        assert_eq!(back.model_catalog, ModelCatalog::chosen(EditionChoice::Auto));
        // Chosen once is chosen on the next save too.
        let (_, again) = saved(back);
        assert!(again.contains("outside_agents = \"v1\"") && again.contains("edition = \"auto\""), "{again}");
        // One switch chosen: only that key is written.
        let mut half = valid_config();
        half.memory = MemoryConfig::default().with_linggen(Mode::V2);
        let (_, text) = saved(half);
        assert!(text.contains("linggen = \"v2\"") && !text.contains("outside_agents"), "{text}");
        // Never chosen: nothing is written, so a default that moves can move.
        let (never, text) = saved(valid_config());
        assert!(!text.contains("[memory]") && !text.contains("[model_catalog]"), "{text}");
        assert!(never.memory.is_unset() && never.model_catalog.is_unset());
    }

    /// The lock: no config type leaves a value out because it equals the
    /// default (`Explicit` is the one way, `config/explicit.rs`). A second
    /// copy of the old `is_default` skip fails here.
    #[test]
    fn a_saved_config_never_drops_a_value_for_equalling_the_default() {
        fn walk(dir: &Path, found: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, found);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                for (n, line) in text.lines().enumerate() {
                    let skips_on_default = line.contains("skip_serializing_if")
                        && (line.contains("is_default") || line.contains("Default::default"));
                    if skips_on_default {
                        found.push(format!("{}:{}", path.display(), n + 1));
                    }
                }
            }
        }
        let mut found = Vec::new();
        walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut found);
        assert!(
            found.is_empty(),
            "a value that equals today's default is still the person's choice; hold it in `Explicit<T>` and skip it with `Explicit::is_unset`:\n{}",
            found.join("\n")
        );
    }

    #[test]
    fn retired_tool_permission_key_is_ignored() {
        use crate::engine::{permission::PermissionMode, EngineConfig, InterfaceMode};

        for (legacy, current, expected) in [
            ("auto", "", PermissionMode::Read),
            ("not_a_mode", "", PermissionMode::Read),
            ("auto", "default_permission_mode = 'chat'", PermissionMode::Chat),
        ] {
            let mut file = toml::Value::try_from(Config::default()).unwrap();
            file["agent"] = toml::from_str(&format!(
                "max_iters = 10\ntool_permission_mode = '{legacy}'\n{current}"
            ))
            .unwrap();
            let cfg: Config = toml::from_str(&toml::to_string(&file).unwrap()).unwrap();
            let engine = EngineConfig::from_app_config(&cfg, Default::default(), InterfaceMode::Web);
            assert_eq!(engine.permission_mode, expected);
            assert!(!toml::to_string(&cfg).unwrap().contains("tool_permission_mode"));
        }
    }

    // Config::default() no longer carries a hardcoded model (built-ins cover
    // that at runtime — see inject_chatgpt_builtin/inject_linggen_cloud), so
    // tests exercising per-model validate() rules push their own.
    fn valid_config() -> Config {
        let mut cfg = Config::default();
        cfg.models.push(ModelConfig {
            id: "test-model".to_string(),
            provider: "openai".to_string(),
            url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o".to_string(),
            api_key: None,
            keep_alive: None,
            context_window: None,
            tags: Vec::new(),
            supports_tools: None,
            auth_mode: None,
            reasoning_effort: None,
            max_output_tokens: None,
            provided_by: None,
            is_builtin: false,
            family_binding: None,
            price: None,
        });
        cfg
    }

    #[test]
    fn an_old_pet_table_still_loads_as_yinyue_settings() {
        let mut v: toml::Value = toml::Value::try_from(Config::default()).unwrap();
        let old: toml::Value = toml::from_str(
            "enabled = false\npet = \"yinyue\"\nmodel = \"m-her\"\nvoice = \"serena\"\nrecall_count = 4\nrecall_min_score = 0.6\n",
        )
        .unwrap();
        v.as_table_mut().unwrap().insert("pet".into(), old);
        let cfg: Config = toml::from_str(&toml::to_string(&v).unwrap()).unwrap();
        assert!(!cfg.pet.enabled);
        assert_eq!(cfg.pet.pet, "yinyue");
        assert_eq!(cfg.pet.model, "m-her");
        assert_eq!(cfg.pet.voice, "serena");
        assert_eq!(cfg.pet.recall_count, 4);
        assert!((cfg.pet.recall_min_score - 0.6).abs() < 1e-6);
        assert!(!cfg.pet.muted);
    }

    /// Upgrade from 2.5.1: the retired Claude-session default
    /// (`routing.claude_default_model`) becomes the default model when the
    /// config named no other, so a person who used only Claude keeps it with
    /// no step of theirs; the key is never written again.
    #[test]
    fn a_claude_session_default_is_read_as_the_native_default() {
        // A 2.5.1 file: the whole default config, its `[routing]` as given.
        let file = |routing: &str| {
            let mut cfg = Config::default();
            cfg.routing.default_models.clear();
            let text = toml::to_string(&cfg).unwrap();
            let (head, tail) = text.split_once("[routing]\n").unwrap();
            let tail = tail.replacen("default_models = []\n", "", 1);
            Config::from_toml(&format!("{head}[routing]\n{routing}{tail}")).unwrap()
        };
        let only = file("claude_default_model = \"claude-sonnet-5-5\"\n");
        assert_eq!(only.routing.default_models, vec!["claude-sonnet-5-5".to_string()]);
        assert!(!toml::to_string(&only).unwrap().contains("claude_default_model"));
        let both = file("default_models = [\"m1\"]\nclaude_default_model = \"claude-sonnet-5-5\"\n");
        assert_eq!(both.routing.default_models, vec!["m1".to_string()]);
        assert!(!toml::to_string(&both).unwrap().contains("claude_default_model"));
    }

    /// Settings → Agents writes Ling's pick to the front of the default chain
    /// and Yinyue's to `[pet] model`; both survive a save and reload.
    #[test]
    fn ling_and_yinyue_models_round_trip() {
        let mut cfg = Config::default();
        cfg.routing.default_models = vec!["m-ling".into(), "m-fallback".into()];
        cfg.pet.model = "m-her".into();
        let text = toml::to_string(&cfg).unwrap();
        assert!(text.contains("[pet]"));
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(
            back.routing.default_models,
            vec!["m-ling".to_string(), "m-fallback".to_string()]
        );
        assert_eq!(back.pet.model, "m-her");
        assert_eq!(back.pet.pet, "yinyue");
    }

    #[test]
    fn a_config_from_before_mute_reads_as_voice_on() {
        let pet: PetConfig = toml::from_str("enabled = true\nvoice = \"vivian\"\n").unwrap();
        assert!(!pet.muted);
        assert!(!Config::default().pet.muted);
    }

    /// Retired routing-policy keys still in old configs load and are ignored.
    #[test]
    fn a_config_with_retired_routing_policies_still_loads() {
        let routing: RoutingConfig = toml::from_str(
            "default_policy = \"local-first\"\ndefault_models = [\"m1\"]\n\n[[policies]]\nname = \"p\"\n\n[[policies.rules]]\nmodel = \"m1\"\nmin_complexity = \"high\"\n",
        )
        .unwrap();
        assert_eq!(routing.default_models, vec!["m1".to_string()]);
        assert!(routing.model_fallback);
    }

    #[test]
    fn retired_auto_fallback_is_ignored_and_never_saved() {
        for old in ["false", "true", "'invalid'"] {
            let mut file = toml::Value::try_from(Config::default()).unwrap();
            file["routing"] = toml::from_str(&format!("auto_fallback = {old}\n")).unwrap();
            let cfg = Config::from_toml(&toml::to_string(&file).unwrap()).unwrap();
            assert!(cfg.routing.model_fallback, "old value {old}");
            assert!(!toml::to_string(&cfg).unwrap().contains("auto_fallback"));
        }
    }

    #[test]
    fn model_fallback_off_survives_save_and_reload() {
        let mut file = toml::Value::try_from(Config::default()).unwrap();
        file["routing"] = toml::from_str("model_fallback = false\n").unwrap();
        let cfg = Config::from_toml(&toml::to_string(&file).unwrap()).unwrap();
        assert!(!cfg.routing.model_fallback);
        let saved = toml::to_string(&cfg).unwrap();
        assert!(!saved.contains("auto_fallback"));
        assert!(!Config::from_toml(&saved).unwrap().routing.model_fallback);
    }

    /// `model_fallback` has one default, `RoutingConfig::default`: a config
    /// made fresh (no file), a file without `[routing]`, and a `[routing]`
    /// without the key all say true. The engine asks the same default (see
    /// `engine::types` tests).
    #[test]
    fn model_fallback_has_one_default() {
        let fresh = Config::default().routing.model_fallback;
        let mut file = toml::Value::try_from(Config::default()).unwrap();
        file.as_table_mut().unwrap().remove("routing");
        let no_table: Config = toml::from_str(&toml::to_string(&file).unwrap()).unwrap();
        let no_key: RoutingConfig = toml::from_str("default_models = [\"m1\"]\n").unwrap();
        assert!(RoutingConfig::default().model_fallback);
        assert!(fresh, "a fresh config");
        assert!(no_table.routing.model_fallback, "a file without [routing]");
        assert!(no_key.model_fallback, "a [routing] without the key");
        // Every routing key: missing from the file means the Rust default.
        let empty: RoutingConfig = toml::from_str("").unwrap();
        assert_eq!(
            toml::to_string(&empty).unwrap(),
            toml::to_string(&RoutingConfig::default()).unwrap()
        );
    }

    #[test]
    fn make_default_keeps_only_the_fallbacks_the_person_set() {
        let mut config = Config::default();
        config.routing.default_models.clear();
        config.make_default("a");
        assert_eq!(config.routing.default_models, ["a"]);
        config.routing.default_models.extend(["b".into(), "c".into()]);
        config.make_default("d");
        assert_eq!(config.routing.default_models, ["d", "b", "c"]);
        config.make_default("b");
        assert_eq!(config.routing.default_models, ["b", "c"]);
        config.make_default("b");
        assert_eq!(config.routing.default_models, ["b", "c"]);
    }

    // ---- Config::validate tests ----

    #[test]
    fn test_validate_default_config() {
        valid_config().validate().unwrap();
    }

    #[test]
    fn test_validate_empty_models_ok() {
        // No explicit models is valid — built-in models (Linggen Cloud,
        // ChatGPT) are always injected by ModelManager at runtime.
        let mut cfg = valid_config();
        cfg.models.clear();
        cfg.validate().unwrap();
    }

    #[test]
    fn test_validate_empty_model_id() {
        let mut cfg = valid_config();
        cfg.models[0].id = "  ".to_string();
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("Model ID cannot be empty"));
    }

    #[test]
    fn test_validate_keep_alive_missing_unit() {
        let mut cfg = valid_config();
        cfg.models[0].keep_alive = Some("10".to_string());
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("needs a time unit"));
    }

    #[test]
    fn test_validate_keep_alive_valid_durations() {
        let mut cfg = valid_config();
        for ka in ["10m", "30s", "1h30m", "0", "-1m", "1.5h"] {
            cfg.models[0].keep_alive = Some(ka.to_string());
            cfg.validate().unwrap();
        }
    }

    #[test]
    fn test_validate_keep_alive_garbage() {
        let mut cfg = valid_config();
        for ka in ["", "abc", "10 m", "10x"] {
            cfg.models[0].keep_alive = Some(ka.to_string());
            assert!(cfg.validate().is_err(), "expected '{ka}' to be rejected");
        }
    }

    #[test]
    fn test_validate_episodic_ttl_zero() {
        let mut cfg = valid_config();
        cfg.agent.episodic_ttl_days = 0;
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("episodic_ttl_days"));
    }

    #[test]
    fn test_default_consolidation_settings() {
        let cfg = valid_config();
        assert_eq!(cfg.agent.episodic_ttl_days, 7);
        cfg.validate().unwrap();
    }

    #[test]
    fn test_validate_duplicate_model_ids() {
        let mut cfg = valid_config();
        let dup = cfg.models[0].clone();
        cfg.models.push(dup);
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("Duplicate model ID"));
    }

    fn priced(input: f64, output: f64) -> Config {
        let mut cfg = valid_config();
        cfg.models[0].price = Some(ModelPrice { input, output, cache_read: None, cache_write: None });
        cfg
    }

    /// Finding 7: a price is real money — every number finite and not
    /// negative, and the error names the model and the field.
    #[test]
    fn a_price_must_be_finite_and_not_negative() {
        priced(2.0, 10.0).validate().unwrap();
        priced(0.0, 0.0).validate().unwrap();
        for (cfg, field) in [
            (priced(-1.0, 10.0), "price.input"),
            (priced(2.0, f64::NAN), "price.output"),
            (priced(f64::INFINITY, 1.0), "price.input"),
        ] {
            let said = cfg.validate().unwrap_err().to_string();
            assert!(said.contains("'test-model'") && said.contains(field), "{said}");
        }
        let mut cfg = priced(1.0, 1.0);
        cfg.models[0].price.as_mut().unwrap().cache_write = Some(-0.5);
        assert!(cfg.validate().unwrap_err().to_string().contains("price.cache_write"));
    }

    /// Finding 7: a price on a model that bills an account is never used.
    #[test]
    fn a_price_on_an_allowance_model_is_an_error() {
        let mut cfg = priced(1.0, 1.0);
        cfg.models[0].provider = "chatgpt".into();
        cfg.models[0].auth_mode = Some("chatgpt_oauth".into());
        let said = cfg.validate().unwrap_err().to_string();
        assert!(said.contains("'test-model'") && said.contains("price"), "{said}");
    }

    /// Strict when writing, lenient when reading: one bad price must never
    /// cost the person the rest of their config. At load it drops that
    /// model's price alone and says so.
    #[test]
    fn a_bad_price_in_the_file_drops_only_that_models_price() {
        let mut cfg = priced(-2.0, 10.0);
        cfg.server.url = Some("127.0.0.1:9999".into());
        let mut second = cfg.models[0].clone();
        second.id = "other".into();
        second.price = Some(ModelPrice { input: 1.0, output: 2.0, cache_read: None, cache_write: None });
        cfg.models.push(second);
        let loaded = Config::from_toml(&toml::to_string(&cfg).unwrap()).expect("the file loads");
        assert_eq!(loaded.server.port(), 9999, "the rest of the config is the person's own");
        assert_eq!(loaded.models.len(), 2);
        assert_eq!(loaded.models[0].price, None, "the bad price is dropped");
        assert_eq!(loaded.models[1].price.as_ref().unwrap().input, 1.0, "a good price is kept");
        assert_eq!(loaded.problems.len(), 1, "{:?}", loaded.problems);
        let said = &loaded.problems[0];
        assert!(said.contains("'test-model'") && said.contains("price.input"), "{said}");
    }

    /// A price on a model that bills an account is dropped the same way.
    #[test]
    fn a_price_on_an_allowance_model_is_dropped_at_load() {
        let mut cfg = priced(1.0, 1.0);
        cfg.models[0].provider = "chatgpt".into();
        cfg.models[0].auth_mode = Some("chatgpt_oauth".into());
        let loaded = Config::from_toml(&toml::to_string(&cfg).unwrap()).expect("loads");
        assert_eq!(loaded.models[0].price, None);
        assert!(loaded.problems[0].contains("'test-model'"), "{:?}", loaded.problems);
    }

    /// A good file has nothing to say.
    #[test]
    fn a_good_price_loads_with_no_problem() {
        let loaded = Config::from_toml(&toml::to_string(&priced(2.0, 10.0)).unwrap()).unwrap();
        assert_eq!(loaded.models[0].price.as_ref().unwrap().input, 2.0);
        assert!(loaded.problems.is_empty());
    }

    /// The older hazard: a file that could not be loaded must never be
    /// written over by defaults. It is set aside, whole, before the save.
    #[test]
    fn a_config_file_that_cannot_load_is_set_aside_before_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = Config::runtime_config_path(Some(dir.path()));
        let broken = b"[server\nurl = \"127.0.0.1:9\" # the person's own models are below\n";
        std::fs::write(&path, broken).unwrap();
        valid_config().save_runtime(Some(dir.path())).unwrap();
        let aside: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with("linggen.runtime.toml.unreadable-"))
            .collect();
        assert_eq!(aside.len(), 1, "{aside:?}");
        assert_eq!(std::fs::read(dir.path().join(&aside[0])).unwrap(), broken, "the original bytes are kept");
        Config::from_toml(&std::fs::read_to_string(&path).unwrap()).expect("the new file loads");
    }

    /// A file that loads is replaced as before — nothing set aside.
    #[test]
    fn a_config_file_that_loads_is_not_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        valid_config().save_runtime(Some(dir.path())).unwrap();
        valid_config().save_runtime(Some(dir.path())).unwrap();
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten()
            .map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert_eq!(names, ["linggen.runtime.toml"], "{names:?}");
    }

    /// `ling status` says what it found: the file that cannot load, and a
    /// copy set aside earlier.
    #[test]
    fn the_health_lines_name_an_unloadable_file_and_its_set_aside_copy() {
        let dir = tempfile::tempdir().unwrap();
        let path = Config::runtime_config_path(Some(dir.path()));
        std::fs::write(&path, "[server").unwrap();
        let lines = Config::health_lines(&[path.clone()]);
        assert!(lines.iter().any(|l| l.contains("could not be read") && l.contains("linggen.runtime.toml")), "{lines:?}");
        valid_config().save_runtime(Some(dir.path())).unwrap();
        let lines = Config::health_lines(&[path]);
        assert!(lines.iter().any(|l| l.contains(".unreadable-")), "{lines:?}");
        assert!(!lines.iter().any(|l| l.contains("could not be read")), "{lines:?}");
    }

    #[test]
    fn test_validate_unknown_provider() {
        let mut cfg = valid_config();
        cfg.models[0].provider = "some_random_provider_xyz".to_string();
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("unknown provider"));
    }

    fn with_live_voice_row() -> Config {
        let mut cfg = valid_config();
        let mut live = cfg.models[0].clone();
        live.id = "voice".into();
        live.provider = "qwen".into();
        live.url = "https://ap-southeast-1.maas.qwencloudapi.com/compatible-mode/v1".into();
        live.model = "qwen-audio-3.1-realtime-flash".into();
        cfg.models.push(live);
        cfg
    }

    /// A live-voice row may sit in the config; it may not be named where a
    /// chat turn runs: the chain, the companion's model, an agent's model.
    #[test]
    fn a_live_voice_row_is_refused_where_a_chat_turn_runs() {
        with_live_voice_row().validate().expect("the row alone is fine");
        let mut cfg = with_live_voice_row();
        cfg.routing.default_models = vec!["voice".into(), "test-model".into()];
        assert!(cfg.validate().unwrap_err().to_string().contains("live-voice"));
        let mut cfg = with_live_voice_row();
        cfg.pet.model = "voice".into();
        assert!(cfg.validate().unwrap_err().to_string().contains("pet.model"));
        let mut cfg = with_live_voice_row();
        cfg.agents.push(AgentSpecRef {
            id: "ling".into(),
            spec_path: "agents/ling.md".into(),
            model: Some("voice".into()),
        });
        assert!(cfg.validate().unwrap_err().to_string().contains("agents[].model"));
        let mut cfg = with_live_voice_row();
        cfg.routing.default_models = vec!["test-model".into()];
        cfg.validate().expect("a chat model in the chain is fine");
    }

    #[test]
    fn chat_models_leave_out_a_live_voice_row() {
        let cfg = with_live_voice_row();
        let ids: Vec<&str> = cfg.chat_models().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["test-model"]);
        assert!(cfg.is_live_voice_row("voice") && !cfg.is_live_voice_row("test-model"));
    }

    #[test]
    fn test_validate_bad_url_scheme() {
        let mut cfg = valid_config();
        cfg.models[0].url = "ftp://example.com".to_string();
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("http://"));
    }

    #[test]
    fn test_validate_port_zero() {
        let mut cfg = valid_config();
        cfg.server.url = Some("127.0.0.1:0".to_string());
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("port must be greater than 0"));
    }

    #[test]
    fn test_validate_max_iters_zero() {
        let mut cfg = valid_config();
        cfg.agent.max_iters = 0;
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("max_iters must be greater than 0"));
    }

    /// A chat turn keeps its 200 steps; a coding session has its own key,
    /// at the most the config allows, which an older config file without it
    /// gets too.
    #[test]
    fn a_coding_session_has_its_own_step_limit_key() {
        let cfg = Config::default();
        assert_eq!((cfg.agent.max_iters, cfg.agent.coding_max_iters), (200, 1000));
        let old: AgentConfig = toml::from_str("max_iters = 5").unwrap();
        assert_eq!(old.coding_max_iters, 1000);
        let set: AgentConfig = toml::from_str("max_iters = 5\ncoding_max_iters = 300").unwrap();
        assert_eq!(set.coding_max_iters, 300);

        let mut cfg = valid_config();
        cfg.agent.coding_max_iters = 0;
        assert!(cfg.validate().unwrap_err().to_string().contains("coding_max_iters must be greater than 0"));
        cfg.agent.coding_max_iters = 1001;
        assert!(cfg.validate().unwrap_err().to_string().contains("coding_max_iters must not exceed 1000"));
    }

    #[test]
    fn test_validate_max_iters_too_large() {
        let mut cfg = valid_config();
        cfg.agent.max_iters = 1001;
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("must not exceed 1000"));
    }

    #[test]
    fn test_validate_openai_provider() {
        let mut cfg = valid_config();
        cfg.models[0].provider = "openai".to_string();
        cfg.validate().unwrap();
    }

    #[test]
    fn test_validate_https_url() {
        let mut cfg = valid_config();
        cfg.models[0].url = "https://api.openai.com/v1".to_string();
        cfg.validate().unwrap();
    }

    // ---- WriteSafetyMode tests ----

    #[test]
    fn test_write_safety_mode_default() {
        assert_eq!(WriteSafetyMode::default(), WriteSafetyMode::Warn);
    }

    #[test]
    fn test_write_safety_mode_serde() {
        let modes = [
            (WriteSafetyMode::Strict, "\"strict\""),
            (WriteSafetyMode::Warn, "\"warn\""),
            (WriteSafetyMode::Off, "\"off\""),
        ];
        for (mode, expected) in &modes {
            let serialized = serde_json::to_string(mode).unwrap();
            assert_eq!(&serialized, expected);
            let deserialized: WriteSafetyMode = serde_json::from_str(expected).unwrap();
            assert_eq!(&deserialized, mode);
        }
    }

    // ---- Config TOML round-trip ----

    /// The shape an existing install has on disk. It must keep working, and
    /// `0.0.0.0` must survive — the field pair is how a user opens the daemon
    /// to their LAN, and silently reverting that to loopback would take the
    /// machine off the network without saying so.
    #[test]
    fn a_legacy_host_port_pair_still_binds_where_it_says() {
        let cfg: ServerConfig = toml::from_str("port = 9527\nhost = \"0.0.0.0\"").unwrap();
        assert_eq!(cfg.addr(), ("0.0.0.0".to_string(), 9527));
    }

    /// …and folds into one field on the next write, saying so rather than
    /// changing shape under the user in silence.
    #[test]
    fn the_legacy_pair_migrates_to_one_field() {
        let mut cfg: ServerConfig = toml::from_str("port = 9600\nhost = \"0.0.0.0\"").unwrap();
        cfg.migrate_legacy_fields();
        assert_eq!(cfg.url.as_deref(), Some("0.0.0.0:9600"));
        assert!(cfg.port.is_none() && cfg.host.is_none());
        assert_eq!(
            toml::to_string(&cfg).unwrap().trim(),
            "url = \"0.0.0.0:9600\""
        );
    }

    /// A GPT generation bump moves each retired built-in to its successor —
    /// defaults and the pet's pin alike — and drops stale oauth copies; a
    /// user's own model under a retired id is theirs and stays.
    #[test]
    fn retired_chatgpt_ids_follow_the_bump() {
        let mut cfg = Config::default();
        cfg.routing.default_models = vec![
            "gpt-5.6-terra".into(),
            "gpt-5.6-luna".into(),
            "gpt-5.6-sol".into(),
        ];
        cfg.pet.model = "gpt-5.6-luna".into();
        cfg.migrate_retired_chatgpt_builtins();
        assert_eq!(
            cfg.routing.default_models,
            vec![
                "gpt-6.1-terra".to_string(),
                "gpt-6-luna".to_string(),
                "gpt-6.2-sol".to_string()
            ]
        );
        assert_eq!(cfg.pet.model, "gpt-6-luna");

        let mut six = Config::default();
        six.routing.default_models = vec!["gpt-6-sol".into()];
        six.pet.model = "gpt-6-sol".into();
        six.migrate_retired_chatgpt_builtins();
        assert_eq!(six.routing.default_models, vec!["gpt-6.2-sol".to_string()]);
        assert_eq!(six.pet.model, "gpt-6.2-sol");

        let mut one = Config::default();
        one.routing.default_models = vec!["gpt-6.1-sol".into()];
        one.migrate_retired_chatgpt_builtins();
        assert_eq!(one.routing.default_models, vec!["gpt-6.2-sol".to_string()]);

        let mut own = valid_config();
        own.models[0].id = "gpt-5.6-sol".into();
        own.routing.default_models = vec!["gpt-5.6-sol".into()];
        own.migrate_retired_chatgpt_builtins();
        assert_eq!(own.routing.default_models, vec!["gpt-5.6-sol".to_string()]);
    }

    /// Defaults and the pet follow a family only to a model that has
    /// answered: a newer Sol the backend lists but that has not answered
    /// leaves them where they are; once it has, they move.
    #[test]
    fn defaults_follow_a_family_only_to_a_model_that_answered() {
        use crate::provider::models::chatgpt_catalog::{tests::measured_list, Listed, Snapshot};
        let mut snap = Snapshot::seed();
        snap.listed = measured_list();
        snap.listed.insert(0, Listed { slug: "gpt-7-sol".into(), ..Listed::default() });
        let mut cfg = Config::default();
        cfg.routing.default_models = vec!["gpt-6.2-sol".into(), "gpt-6.1-sol".into()];
        cfg.pet.model = "gpt-6.2-sol".into();
        cfg.follow_chatgpt_catalog(&snap);
        assert_eq!(cfg.routing.default_models, vec!["gpt-6.2-sol".to_string()]);
        assert_eq!(cfg.pet.model, "gpt-6.2-sol");

        let sol = snap.family_mut("sol").unwrap();
        sol.previous = Some("gpt-6.2-sol".into());
        sol.current = "gpt-7-sol".into();
        assert!(cfg.follow_chatgpt_catalog(&snap));
        assert_eq!(cfg.routing.default_models, vec!["gpt-7-sol".to_string()]);
        assert_eq!(cfg.pet.model, "gpt-7-sol");
        assert!(!cfg.follow_chatgpt_catalog(&snap), "followed once");

        // Staying on the previous Sol brings them back.
        snap.family_mut("sol").unwrap().stay = true;
        assert!(cfg.follow_chatgpt_catalog(&snap));
        assert_eq!(cfg.routing.default_models, vec!["gpt-6.2-sol".to_string()]);
    }

    /// A model the person added on the ChatGPT sign-in under a listed slug
    /// no family has been on is theirs: kept, and a default naming it stays.
    #[test]
    fn a_users_chatgpt_model_under_a_newer_listed_slug_is_kept() {
        use crate::provider::models::chatgpt_catalog::{tests::measured_list, Listed, Snapshot};
        let mut snap = Snapshot::seed();
        snap.listed = measured_list();
        snap.listed.insert(0, Listed { slug: "gpt-7-sol".into(), ..Listed::default() });
        let mut cfg = valid_config();
        cfg.models[0].id = "gpt-7-sol".into();
        cfg.models[0].auth_mode = Some("chatgpt_oauth".into());
        cfg.routing.default_models = vec!["gpt-7-sol".into()];
        assert!(!cfg.follow_chatgpt_catalog(&snap));
        assert_eq!(cfg.models.len(), 1);
        assert_eq!(cfg.routing.default_models, vec!["gpt-7-sol".to_string()]);
    }

    /// A retired Linggen Cloud id persisted as a default or a pet pin follows
    /// the rename; a user's own model under that id is theirs and stays.
    #[test]
    fn retired_cloud_ids_follow_the_rename() {
        let mut cfg = Config::default();
        cfg.routing.default_models = vec!["deepseek-v4-flash".into(), "deepseek-flash".into()];
        cfg.pet.model = "deepseek-v4-flash".into();
        cfg.migrate_retired_cloud_ids();
        assert_eq!(
            cfg.routing.default_models,
            vec!["deepseek-flash".to_string()]
        );
        assert_eq!(cfg.pet.model, "deepseek-flash");

        let mut own = valid_config();
        own.models[0].id = "deepseek-v4-flash".into();
        own.routing.default_models = vec!["deepseek-v4-flash".into()];
        own.migrate_retired_cloud_ids();
        assert_eq!(
            own.routing.default_models,
            vec!["deepseek-v4-flash".to_string()]
        );
    }

    /// `url` is authored, so it wins outright — a stale legacy pair left in the
    /// file must not quietly outrank the field the user just set.
    #[test]
    fn url_wins_over_a_leftover_pair() {
        let cfg: ServerConfig =
            toml::from_str("url = \"0.0.0.0:9600\"\nport = 9527\nhost = \"127.0.0.1\"").unwrap();
        assert_eq!(cfg.addr(), ("0.0.0.0".to_string(), 9600));
    }

    /// A scheme is accepted and dropped: the same address is written with one
    /// everywhere it is *consumed*, so pasting that form must not silently
    /// fall back to the default port.
    #[test]
    fn a_scheme_is_accepted_and_ignored() {
        assert_eq!(
            parse_addr("http://0.0.0.0:9527"),
            Some(("0.0.0.0".into(), 9527))
        );
        assert_eq!(
            parse_addr("http://192.168.1.5:9527/"),
            Some(("192.168.1.5".into(), 9527))
        );
        assert_eq!(
            parse_addr("127.0.0.1:9527"),
            Some(("127.0.0.1".into(), 9527))
        );
        // Not an address: no port, empty host, unparseable port.
        assert_eq!(parse_addr("127.0.0.1"), None);
        assert_eq!(parse_addr(":9527"), None);
        assert_eq!(parse_addr("127.0.0.1:http"), None);
    }

    /// An unusable `url` falls back rather than panicking or binding somewhere
    /// unintended — a typo should cost a warning and the default, not the LAN.
    #[test]
    fn a_malformed_url_falls_back_to_the_default() {
        let cfg: ServerConfig = toml::from_str("url = \"nonsense\"").unwrap();
        assert_eq!(cfg.addr(), ("127.0.0.1".to_string(), DEFAULT_PORT));
    }

    /// An empty `[server]` is the fresh-install case.
    #[test]
    fn an_empty_server_table_is_the_default_address() {
        let cfg: ServerConfig = toml::from_str("").unwrap();
        assert_eq!(cfg.addr(), ("127.0.0.1".to_string(), DEFAULT_PORT));
    }

    #[test]
    fn test_config_toml_roundtrip() {
        let cfg = Config::default();
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.models.len(), cfg.models.len());
        assert_eq!(parsed.server.addr(), cfg.server.addr());
        assert_eq!(parsed.agent.max_iters, cfg.agent.max_iters);
    }

    #[test]
    fn explicit_daily_session_choices_survive_saving() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        let mut save_and_read = |choice: Option<bool>| {
            cfg.agent.daily_session = choice;
            let path = cfg.save_runtime(Some(dir.path())).unwrap();
            let text = fs::read_to_string(path).unwrap();
            match choice {
                Some(value) => assert!(
                    text.contains(&format!("daily_session = {value}")),
                    "explicit choice was omitted"
                ),
                None => assert!(!text.contains("daily_session")),
            }
            let saved: Config = toml::from_str(&text).unwrap();
            assert_eq!(saved.agent.daily_session, choice);
            saved.agent
        };

        let off = save_and_read(Some(false));
        assert!(!off.daily_session_with_default(false));
        assert!(!off.daily_session_with_default(true));
        assert!(save_and_read(Some(true)).daily_session_with_default(false));
        let unset = save_and_read(None);
        assert!(!unset.daily_session_with_default(false));
        assert!(unset.daily_session_with_default(true));
    }

    /// Memory is the store this engine hosts; there is no address to set. A
    /// config from an older engine that still names one loads, the key
    /// ignored, and is not written back.
    #[test]
    fn an_old_memory_address_in_the_config_is_ignored() {
        let old: AgentConfig =
            toml::from_str("max_iters = 5\nling_mem_url = \"http://127.0.0.1:4100\"").unwrap();
        assert_eq!(old.max_iters, 5);
        let cfg = Config {
            agent: old,
            ..Config::default()
        };
        let text = toml::to_string_pretty(&cfg).unwrap();
        assert!(!text.contains("ling_mem_url"), "{text}");
    }
}

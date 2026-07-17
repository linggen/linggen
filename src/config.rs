use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone)]
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
    /// Default working folder for new sessions. Defaults to `~` if not set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_path: Option<String>,
    /// Desktop pet / companion (Yinyue) settings — see [`PetConfig`].
    /// Surfaced in Settings → General → Pet.
    #[serde(default)]
    pub pet: PetConfig,
}

/// Settings for the desktop pet / companion (Yinyue today). The recall fields
/// are intentionally separate from the global `agent.memory_*` knobs: a
/// companion does many small turns, so her recall stays tighter than a coding
/// session's.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PetConfig {
    /// Master switch. When false the pet doesn't render and her event-reactive
    /// watch loop stays silent.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Which pet/avatar to show. One id per shipped model; "yinyue" today.
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

#[derive(Debug, Serialize, Deserialize, Clone)]
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
    #[serde(default)]
    pub auth_mode: Option<String>,
    /// Reasoning effort level: "low", "medium", "high".
    /// Translates to provider-specific parameters:
    /// - OpenAI/o-series/GPT-5: `reasoning_effort`
    /// - Gemini 2.5: `thinkingConfig.thinkingBudget`
    /// - Others: ignored (no-op)
    #[serde(default)]
    pub reasoning_effort: Option<String>,
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
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentSpecRef {
    pub id: String,
    pub spec_path: String,
    pub model: Option<String>,
}

/// The daemon's default listen port. Single source of truth — every engine
/// consumer of the default (CLI, config default, install/autostart scripts,
/// clients) resolves to this rather than a scattered literal. Override per
/// install via `[server].port` in linggen.toml or `--port`.
pub const DEFAULT_PORT: u16 = 9898;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerConfig {
    #[serde(default = "default_server_port")]
    pub port: u16,
    #[serde(default = "default_server_host")]
    pub host: String,
}

fn default_server_port() -> u16 {
    DEFAULT_PORT
}

fn default_server_host() -> String {
    "127.0.0.1".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentConfig {
    pub max_iters: usize,
    #[serde(default)]
    pub write_safety_mode: WriteSafetyMode,
    /// Legacy permission mode (ask/auto/accept_edits). Use `default_permission_mode` instead.
    #[serde(default)]
    pub tool_permission_mode: ToolPermissionMode,
    /// New permission mode (chat/read/edit/admin). Takes precedence over `tool_permission_mode`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_permission_mode: Option<crate::engine::permission::PermissionMode>,
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

    /// Base URL of the local `ling-mem` HTTP daemon. The engine's built-in
    /// `Memory_query` / `Memory_write` tools dispatch here, and the `dream`
    /// mission reads `episodic_ttl_days` from `<url>/api/config`. Default
    /// is the daemon's own default port — change only if you ran `ling-mem
    /// start` against a different `--port`, or pointed it at a remote
    /// host. Trailing slash optional; no path segment.
    #[serde(default = "default_ling_mem_url")]
    pub ling_mem_url: String,
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

fn default_ling_mem_url() -> String {
    "http://127.0.0.1:9888".to_string()
}

impl AgentConfig {
    /// Resolve effective permission mode — new field takes precedence, falls back to legacy.
    pub fn effective_permission_mode(&self) -> crate::engine::permission::PermissionMode {
        use crate::engine::permission::PermissionMode;
        if let Some(ref mode) = self.default_permission_mode {
            return mode.clone();
        }
        // Convert legacy mode
        match self.tool_permission_mode {
            ToolPermissionMode::Ask => PermissionMode::Read,
            ToolPermissionMode::AcceptEdits => PermissionMode::Edit,
            ToolPermissionMode::Auto => PermissionMode::Admin,
        }
    }
}

fn default_max_delegation_depth() -> usize {
    2
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WriteSafetyMode {
    Strict,
    Warn,
    Off,
}

impl Default for WriteSafetyMode {
    fn default() -> Self {
        // User-selected default for this repo: warn (allow write, but emit warnings).
        WriteSafetyMode::Warn
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermissionMode {
    Ask,
    Auto,
    /// Auto-approve Write/Edit but still prompt for Bash and web tools.
    AcceptEdits,
}

impl Default for ToolPermissionMode {
    fn default() -> Self {
        ToolPermissionMode::Ask
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct LoggingConfig {
    pub level: Option<String>,
    pub directory: Option<String>,
    pub retention_days: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct RoutingConfig {
    #[serde(default)]
    pub default_policy: Option<String>,
    #[serde(default)]
    pub policies: Vec<RoutingPolicy>,
    /// Ordered list of model IDs selected as defaults by the user.
    /// The first model in the list is the primary default; others are fallbacks.
    #[serde(default)]
    pub default_models: Vec<String>,
    /// When true, automatically try the next model on transient errors
    /// (timeout, rate limit, 502/503, connection failures). Default: true.
    #[serde(default = "default_true")]
    pub auto_fallback: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RoutingPolicy {
    pub name: String,
    #[serde(default)]
    pub rules: Vec<RoutingRule>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RoutingRule {
    pub model: String,
    #[serde(default)]
    pub priority: u32,
    #[serde(default)]
    pub min_complexity: Option<ComplexityLevel>,
    #[serde(default)]
    pub max_complexity: Option<ComplexityLevel>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ComplexityLevel {
    Low,
    Medium,
    High,
}

impl Config {
    /// Resolve home_path to an absolute PathBuf. Defaults to `~`.
    pub fn resolved_home_path(&self) -> PathBuf {
        if let Some(ref p) = self.home_path {
            if p.starts_with("~/") || p == "~" {
                let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
                if p == "~" { home } else { home.join(&p[2..]) }
            } else {
                PathBuf::from(p)
            }
        } else {
            dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
        }
    }

    pub fn load_with_path() -> Result<(Self, Option<PathBuf>)> {
        let mut candidates = Vec::new();

        if let Ok(explicit) = std::env::var("LINGGEN_CONFIG") {
            candidates.push(PathBuf::from(explicit));
        }

        // ~/.linggen/config/
        let cfg_dir = crate::paths::config_dir();
        candidates.push(cfg_dir.join("linggen.runtime.toml"));
        candidates.push(cfg_dir.join("linggen.toml"));

        for path in candidates {
            if path.exists() {
                let content = fs::read_to_string(&path)?;
                let mut config: Config = toml::from_str(&content)?;
                config.migrate_retired_chatgpt_builtins();
                return Ok((config, Some(path)));
            }
        }

        Ok((Config::default(), None))
    }

    /// Migrate persisted state left behind by a ChatGPT built-in generation
    /// bump: drop stale copies of a retired built-in (same id + chatgpt_oauth
    /// — a custom API-key entry that happens to reuse the id is kept), and
    /// re-point routing defaults at the current built-in when the retired id
    /// no longer resolves to any configured model.
    fn migrate_retired_chatgpt_builtins(&mut self) {
        use crate::provider::models::{CHATGPT_BUILTIN_MODEL_ID, CHATGPT_RETIRED_MODEL_IDS};
        let retired = |id: &str| CHATGPT_RETIRED_MODEL_IDS.contains(&id);

        self.models.retain(|m| {
            !(retired(&m.id) && m.auth_mode.as_deref() == Some("chatgpt_oauth"))
        });

        let surviving: std::collections::HashSet<&str> =
            self.models.iter().map(|m| m.id.as_str()).collect();
        for dm in &mut self.routing.default_models {
            if retired(dm) && !surviving.contains(dm.as_str()) {
                *dm = CHATGPT_BUILTIN_MODEL_ID.to_string();
            }
        }
        let mut seen = std::collections::HashSet::new();
        self.routing.default_models.retain(|id| seen.insert(id.clone()));
    }

    pub fn runtime_config_path(config_dir: Option<&Path>) -> PathBuf {
        if let Some(dir) = config_dir {
            return dir.join("linggen.runtime.toml");
        }
        crate::paths::config_dir().join("linggen.runtime.toml")
    }

    pub fn save_runtime(&self, config_dir: Option<&Path>) -> Result<PathBuf> {
        let path = Self::runtime_config_path(config_dir);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        fs::write(&path, content)?;
        Ok(path)
    }

    pub fn validate(&self) -> Result<()> {
        // No `models.is_empty()` check: ModelManager always injects the
        // built-in models (Linggen Cloud, ChatGPT) at runtime regardless of
        // what's configured here, so an empty list is a valid "use only the
        // built-ins" config, not a broken one.
        let mut seen_ids = std::collections::HashSet::new();
        for model in &self.models {
            if model.id.trim().is_empty() {
                anyhow::bail!("Model ID cannot be empty");
            }
            if !seen_ids.insert(&model.id) {
                anyhow::bail!("Duplicate model ID: {}", model.id);
            }
            if model.model.trim().is_empty() {
                anyhow::bail!(
                    "Model '{}' has an empty model name. Set the 'model' field to the actual model name (e.g. gemini-2.0-flash).",
                    model.id
                );
            }
            // Validate provider is known.
            let known_providers = ["ollama", "openai", "chatgpt", "anthropic", "gemini", "groq", "deepseek", "openrouter", "github"];
            if !known_providers.contains(&model.provider.as_str()) {
                anyhow::bail!(
                    "Model '{}' has unknown provider '{}'. Known providers: {}",
                    model.id,
                    model.provider,
                    known_providers.join(", ")
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
        }
        if self.server.port == 0 {
            anyhow::bail!("Server port must be greater than 0");
        }
        if self.agent.max_iters == 0 {
            anyhow::bail!("Agent max_iters must be greater than 0");
        }
        if self.agent.max_iters > 1000 {
            anyhow::bail!("Agent max_iters must not exceed 1000");
        }
        // A 0-day TTL would evict episodic rows immediately on the next
        // dream pass, before the user has had a chance to inspect them.
        if self.agent.episodic_ttl_days == 0 {
            anyhow::bail!(
                "Agent episodic_ttl_days must be greater than 0"
            );
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
        let url = self.agent.ling_mem_url.trim();
        if url.is_empty() {
            anyhow::bail!("Agent ling_mem_url must not be empty");
        }
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            anyhow::bail!(
                "Agent ling_mem_url must start with http:// or https:// (got {url})"
            );
        }
        // Warn (log) if default_models references non-existent model IDs.
        // Built-in models (injected at ModelManager build) are valid defaults.
        for dm in &self.routing.default_models {
            if dm == crate::provider::models::LINGGEN_CLOUD_MODEL_ID
                || crate::provider::models::CHATGPT_BUILTIN_MODEL_IDS.contains(&dm.as_str())
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

impl Default for Config {
    fn default() -> Self {
        Self {
            // No hardcoded models — the built-in Linggen Cloud (deepseek-v4-flash)
            // and ChatGPT (CHATGPT_BUILTIN_MODEL_ID) models are always injected by ModelManager
            // at runtime (see inject_linggen_cloud / inject_chatgpt_builtin in
            // provider/models.rs), so a fresh install needs nothing here.
            models: Vec::new(),
            server: ServerConfig { port: DEFAULT_PORT, host: default_server_host() },
            agent: AgentConfig {
                max_iters: 200,
                write_safety_mode: WriteSafetyMode::default(),
                tool_permission_mode: ToolPermissionMode::default(),
                default_permission_mode: None,
                prompt_loop_breaker: None,
                max_delegation_depth: default_max_delegation_depth(),
                compact_threshold: None,
                episodic_ttl_days: default_episodic_ttl_days(),
                memory_inject_min_score: default_memory_inject_min_score(),
                memory_recall_count: default_memory_recall_count(),
                ling_mem_url: default_ling_mem_url(),
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
            home_path: None,
            pet: PetConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            provided_by: None,
            is_builtin: false,
        });
        cfg
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

    #[test]
    fn test_validate_unknown_provider() {
        let mut cfg = valid_config();
        cfg.models[0].provider = "some_random_provider_xyz".to_string();
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("unknown provider"));
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
        cfg.server.port = 0;
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

    #[test]
    fn test_config_toml_roundtrip() {
        let cfg = Config::default();
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.models.len(), cfg.models.len());
        assert_eq!(parsed.server.port, cfg.server.port);
        assert_eq!(parsed.agent.max_iters, cfg.agent.max_iters);
    }
}

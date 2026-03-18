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
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentSpecRef {
    pub id: String,
    pub spec_path: String,
    pub model: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentSpec {
    pub name: String,
    pub description: String,
    pub tools: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub personality: Option<String>,
}


impl AgentSpec {
    pub fn from_markdown_content(content: &str) -> Result<(Self, String)> {
        if !content.starts_with("---") {
            anyhow::bail!("Agent spec must start with YAML frontmatter (---)");
        }
        let parts: Vec<&str> = content.splitn(3, "---").collect();
        if parts.len() < 3 {
            anyhow::bail!("Agent spec missing closing frontmatter delimiter (---)");
        }
        let spec: AgentSpec = serde_yml::from_str(parts[1])?;
        let system_prompt = parts[2].trim().to_string();
        Ok((spec, system_prompt))
    }

    pub fn from_markdown(path: &Path) -> Result<(Self, String)> {
        let content = fs::read_to_string(path)?;
        Self::from_markdown_content(&content)
            .map_err(|e| anyhow::anyhow!("Agent spec at {:?} is invalid: {}", path, e))
    }

}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerConfig {
    pub port: u16,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentConfig {
    pub max_iters: usize,
    #[serde(default)]
    pub write_safety_mode: WriteSafetyMode,
    #[serde(default)]
    pub tool_permission_mode: ToolPermissionMode,
    #[serde(default)]
    pub prompt_loop_breaker: Option<String>,
    #[serde(default = "default_max_delegation_depth")]
    pub max_delegation_depth: usize,
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
                let config: Config = toml::from_str(&content)?;
                return Ok((config, Some(path)));
            }
        }

        Ok((Config::default(), None))
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
        if self.models.is_empty() {
            anyhow::bail!("At least one model must be configured");
        }
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
            let known_providers = ["ollama", "openai", "chatgpt", "gemini", "groq", "deepseek", "openrouter", "github"];
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
        // Warn (log) if default_models references non-existent model IDs.
        for dm in &self.routing.default_models {
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
            models: vec![ModelConfig {
                id: "qwen3.5:35b".to_string(),
                provider: "ollama".to_string(),
                url: "http://127.0.0.1:11434".to_string(),
                model: "qwen3.5:35b".to_string(),
                api_key: None,
                keep_alive: Some("10m".to_string()),
                context_window: None,
                tags: Vec::new(),
                supports_tools: None,
                auth_mode: None,
                reasoning_effort: None,
            }],
            server: ServerConfig { port: 9898 },
            agent: AgentConfig {
                max_iters: 200,
                write_safety_mode: WriteSafetyMode::default(),
                tool_permission_mode: ToolPermissionMode::default(),
                prompt_loop_breaker: None,
                max_delegation_depth: default_max_delegation_depth(),
            },
            logging: LoggingConfig {
                level: None,
                directory: None,
                retention_days: None,
            },
            agents: Vec::new(),
            routing: RoutingConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> Config {
        Config::default()
    }

    // ---- Config::validate tests ----

    #[test]
    fn test_validate_default_config() {
        valid_config().validate().unwrap();
    }

    #[test]
    fn test_validate_empty_models() {
        let mut cfg = valid_config();
        cfg.models.clear();
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("At least one model"));
    }

    #[test]
    fn test_validate_empty_model_id() {
        let mut cfg = valid_config();
        cfg.models[0].id = "  ".to_string();
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("Model ID cannot be empty"));
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
        cfg.models[0].provider = "anthropic".to_string();
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

    // ---- AgentSpec::from_markdown_content tests ----

    #[test]
    fn test_agent_spec_from_markdown_valid() {
        let md = r#"---
name: ling
description: General-purpose assistant
tools:
  - Read
  - Write
  - Bash
---
You are a helpful assistant."#;
        let (spec, prompt) = AgentSpec::from_markdown_content(md).unwrap();
        assert_eq!(spec.name, "ling");
        assert_eq!(spec.description, "General-purpose assistant");
        assert_eq!(spec.tools, vec!["Read", "Write", "Bash"]);
        assert_eq!(prompt, "You are a helpful assistant.");
    }

    #[test]
    fn test_agent_spec_missing_frontmatter() {
        let md = "Just a regular markdown file";
        let err = AgentSpec::from_markdown_content(md).unwrap_err();
        assert!(err.to_string().contains("must start with YAML frontmatter"));
    }

    #[test]
    fn test_agent_spec_missing_closing_delimiter() {
        let md = "---\nname: test\ndescription: test\ntools: []\n";
        let err = AgentSpec::from_markdown_content(md).unwrap_err();
        assert!(err.to_string().contains("missing closing frontmatter"));
    }

    #[test]
    fn test_agent_spec_ignores_unknown_fields() {
        // serde should ignore unknown fields gracefully — this test
        // verifies that basic specs still parse.
        let md = r#"---
name: coder
description: Implementation agent
tools: [Read, Write, Edit]
---
Write code."#;
        let (spec, _) = AgentSpec::from_markdown_content(md).unwrap();
        assert_eq!(spec.name, "coder");
        assert_eq!(spec.tools, vec!["Read", "Write", "Edit"]);
    }

    #[test]
    fn test_agent_spec_with_personality() {
        let md = r#"---
name: ling
description: Personal assistant
tools: ["*"]
personality: |
  Concise and direct.
  Confident but honest.
---
You are Ling."#;
        let (spec, prompt) = AgentSpec::from_markdown_content(md).unwrap();
        assert_eq!(spec.name, "ling");
        assert!(spec.personality.is_some());
        assert!(spec.personality.as_ref().unwrap().contains("Concise and direct"));
        assert_eq!(prompt, "You are Ling.");
    }

    #[test]
    fn test_agent_spec_without_personality() {
        let md = r#"---
name: test
description: No personality
tools: [Read]
---
Prompt."#;
        let (spec, _) = AgentSpec::from_markdown_content(md).unwrap();
        assert!(spec.personality.is_none());
    }

    #[test]
    fn test_agent_spec_ignores_unknown_frontmatter_fields() {
        let md = r#"---
name: ling
description: Lead agent
tools: [Read, Glob]
idle_prompt: "Some old field"
idle_interval_secs: 60
---
You are the lead."#;
        let (spec, _) = AgentSpec::from_markdown_content(md).unwrap();
        assert_eq!(spec.name, "ling");
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

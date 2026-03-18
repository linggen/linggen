use crate::config::{ComplexityLevel, ModelConfig, RoutingConfig};

/// Signals used to determine complexity for routing decisions.
pub struct ComplexitySignal {
    pub estimated_tokens: Option<usize>,
    pub tool_depth: Option<usize>,
    pub _skill_model_hint: Option<String>,
}

impl ComplexitySignal {
    pub fn level(&self) -> ComplexityLevel {
        if let Some(tokens) = self.estimated_tokens {
            if tokens > 4000 {
                return ComplexityLevel::High;
            }
            if tokens > 1500 {
                return ComplexityLevel::Medium;
            }
        }
        if let Some(depth) = self.tool_depth {
            if depth > 3 {
                return ComplexityLevel::High;
            }
            if depth > 1 {
                return ComplexityLevel::Medium;
            }
        }
        ComplexityLevel::Low
    }
}

/// Resolve a model ID using routing policies.
///
/// Looks up the named policy (or `default_policy` if none specified),
/// applies built-in or custom routing rules, and returns the first
/// matching model that exists in `available_models`.
pub fn resolve_model(
    routing: &RoutingConfig,
    policy_name: Option<&str>,
    signal: &ComplexitySignal,
    available_models: &[ModelConfig],
) -> Option<String> {
    if available_models.is_empty() {
        return None;
    }

    let effective_policy = policy_name.or(routing.default_policy.as_deref());
    let Some(policy) = effective_policy else {
        return None;
    };

    // Built-in policies
    match policy {
        "local-first" => return resolve_builtin_local_first(available_models),
        "cloud-first" => return resolve_builtin_cloud_first(available_models),
        _ => {}
    }

    // Custom policy: look up by name in routing.policies
    let custom = routing.policies.iter().find(|p| p.name == policy)?;
    let complexity = signal.level();

    let mut rules = custom.rules.clone();
    rules.sort_by_key(|r| r.priority);

    let model_ids: std::collections::HashSet<&str> =
        available_models.iter().map(|m| m.id.as_str()).collect();

    for rule in &rules {
        if !model_ids.contains(rule.model.as_str()) {
            continue;
        }
        if let Some(min) = rule.min_complexity {
            if complexity < min {
                continue;
            }
        }
        if let Some(max) = rule.max_complexity {
            if complexity > max {
                continue;
            }
        }
        return Some(rule.model.clone());
    }

    None
}

/// Built-in: prefer Ollama (local) models, fall back to non-Ollama (cloud).
fn resolve_builtin_local_first(models: &[ModelConfig]) -> Option<String> {
    if let Some(m) = models.iter().find(|m| m.provider == "ollama") {
        return Some(m.id.clone());
    }
    models.first().map(|m| m.id.clone())
}

/// Built-in: prefer non-Ollama (cloud) models, fall back to local.
fn resolve_builtin_cloud_first(models: &[ModelConfig]) -> Option<String> {
    if let Some(m) = models.iter().find(|m| m.provider != "ollama") {
        return Some(m.id.clone());
    }
    models.first().map(|m| m.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{RoutingPolicy, RoutingRule};

    fn ollama_model(id: &str) -> ModelConfig {
        ModelConfig {
            id: id.to_string(),
            provider: "ollama".to_string(),
            url: "http://localhost:11434".to_string(),
            model: "test".to_string(),
            api_key: None,
            keep_alive: None,
            context_window: None,
            tags: Vec::new(),
            supports_tools: None,
            auth_mode: None,
            reasoning_effort: None,
        }
    }

    fn openai_model(id: &str) -> ModelConfig {
        ModelConfig {
            id: id.to_string(),
            provider: "openai".to_string(),
            url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4".to_string(),
            api_key: Some("key".to_string()),
            keep_alive: None,
            context_window: None,
            tags: Vec::new(),
            supports_tools: None,
            auth_mode: None,
            reasoning_effort: None,
        }
    }

    fn empty_signal() -> ComplexitySignal {
        ComplexitySignal {
            estimated_tokens: None,
            tool_depth: None,
            _skill_model_hint: None,
        }
    }

    #[test]
    fn local_first_prefers_ollama() {
        let models = vec![openai_model("cloud"), ollama_model("local")];
        let routing = RoutingConfig {
            default_policy: Some("local-first".to_string()),
            ..Default::default()
        };
        let result = resolve_model(&routing, None, &empty_signal(), &models);
        assert_eq!(result, Some("local".to_string()));
    }

    #[test]
    fn cloud_first_prefers_openai() {
        let models = vec![ollama_model("local"), openai_model("cloud")];
        let routing = RoutingConfig {
            default_policy: Some("cloud-first".to_string()),
            ..Default::default()
        };
        let result = resolve_model(&routing, None, &empty_signal(), &models);
        assert_eq!(result, Some("cloud".to_string()));
    }

    #[test]
    fn custom_policy_respects_priority_and_complexity() {
        let models = vec![ollama_model("small"), openai_model("big")];
        let routing = RoutingConfig {
            default_policy: Some("custom".to_string()),
            policies: vec![RoutingPolicy {
                name: "custom".to_string(),
                rules: vec![
                    RoutingRule {
                        model: "small".to_string(),
                        priority: 1,
                        min_complexity: None,
                        max_complexity: Some(ComplexityLevel::Medium),
                    },
                    RoutingRule {
                        model: "big".to_string(),
                        priority: 2,
                        min_complexity: Some(ComplexityLevel::High),
                        max_complexity: None,
                    },
                ],
            }],
            ..Default::default()
        };

        // Low complexity -> small
        let low = ComplexitySignal {
            estimated_tokens: Some(100),
            tool_depth: None,
            _skill_model_hint: None,
        };
        assert_eq!(resolve_model(&routing, None, &low, &models), Some("small".to_string()));

        // High complexity -> big
        let high = ComplexitySignal {
            estimated_tokens: Some(5000),
            tool_depth: None,
            _skill_model_hint: None,
        };
        assert_eq!(resolve_model(&routing, None, &high, &models), Some("big".to_string()));
    }

    #[test]
    fn no_policy_returns_none() {
        let models = vec![ollama_model("local")];
        let routing = RoutingConfig::default();
        assert_eq!(resolve_model(&routing, None, &empty_signal(), &models), None);
    }
}

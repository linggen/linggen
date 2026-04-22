use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tracing::info;

use super::tools::ToolResult;

fn default_param_type() -> String {
    "string".to_string()
}

fn default_timeout() -> u64 {
    30000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillParamDef {
    #[serde(rename = "type", default = "default_param_type")]
    pub param_type: String,
    #[serde(default)]
    pub required: bool,
    pub default: Option<Value>,
    #[serde(default)]
    pub description: String,
    /// For array types: schema of each item. If omitted, defaults to `{"type": "object"}`.
    #[serde(default)]
    pub items: Option<Value>,
}

/// How a skill-declared tool is dispatched. Determined by which frontmatter
/// fields are populated:
///
/// - `cmd: "..."`   → `Shell` (shell execution, the classic skill tool)
/// - `endpoint: ...` → `Http` (POST to the skill's daemon; requires the
///   skill to declare a `daemon:` block)
/// - neither        → `Data` (no side effect; args surface as a
///   `content_block` event for app skill UIs)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillToolKind {
    Shell,
    Http,
    Data,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillToolDef {
    pub name: String,
    pub description: String,
    /// Shell command to execute. If empty *and* `endpoint` is also empty,
    /// the tool is a **data tool** — args surface as a `content_block`
    /// event for app skill UIs without running anything.
    #[serde(default)]
    pub cmd: String,
    /// HTTP endpoint path on the skill's daemon, e.g. `/api/memory/search`.
    /// Present makes this an **HTTP tool** — Linggen POSTs the args as JSON
    /// to `http://127.0.0.1:<daemon.port>{endpoint}`. The owning skill
    /// must declare a `daemon:` block (see `doc/skill-spec.md`).
    #[serde(default)]
    pub endpoint: Option<String>,
    /// Permission mode required to invoke this tool: `"read"` | `"edit"` |
    /// `"admin"`. Applies to HTTP and shell tools. Data tools ignore it
    /// (no side effect to gate). Absent → defaults to `"admin"` so
    /// unclassified writes get the strict default.
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub args: HashMap<String, SkillParamDef>,
    #[serde(default)]
    pub returns: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    /// Name of the skill that declared this tool. Set at skill-load time so
    /// dispatch can resolve the daemon (via `SkillManager`) without another
    /// lookup. Not serialized — populated from the containing skill's name.
    #[serde(skip)]
    pub skill_name: Option<String>,
    /// Directory containing the skill file; set at load time.
    #[serde(skip)]
    pub skill_dir: Option<PathBuf>,
}

impl SkillToolDef {
    /// Classify how this tool should be dispatched based on which fields
    /// the skill author filled in.
    pub fn kind(&self) -> SkillToolKind {
        if self.endpoint.is_some() {
            SkillToolKind::Http
        } else if !self.cmd.is_empty() {
            SkillToolKind::Shell
        } else {
            SkillToolKind::Data
        }
    }
}

impl SkillToolDef {
    pub fn execute(&self, args: &Value, workspace_root: &Path) -> Result<ToolResult> {
        let obj = args.as_object();

        // Validate required args.
        for (name, param) in &self.args {
            if param.required {
                let has_arg = obj.map(|o| o.contains_key(name)).unwrap_or(false);
                if !has_arg {
                    anyhow::bail!(
                        "{} call missing required argument '{}'. Description: {}. \
                         Retry with the argument populated.",
                        self.name,
                        name,
                        param.description.trim_end_matches('.'),
                    );
                }
                // For object/array types, also reject empty values — common
                // mistake where the model calls the tool as a "trigger" with
                // {} or []. Forcing a re-call with content is better than
                // silently emitting an empty update.
                if param.param_type == "object" || param.param_type == "array" {
                    if let Some(val) = obj.and_then(|o| o.get(name)) {
                        let is_empty = match val {
                            Value::Object(m) => m.is_empty(),
                            Value::Array(a) => a.is_empty(),
                            _ => false,
                        };
                        if is_empty {
                            anyhow::bail!(
                                "{} call has empty '{}' ({}). {}. Retry with the full payload.",
                                self.name,
                                name,
                                if param.param_type == "object" { "{}" } else { "[]" },
                                param.description.trim_end_matches('.'),
                            );
                        }
                    }
                }
            }
        }

        // Data tool: no cmd → return args as JSON. The value is in the
        // content_block event (tool name + args), not the return value.
        if self.cmd.is_empty() {
            // Reject calls where every argument is missing or empty — the
            // tool is being used as a "trigger" with no payload, which the
            // consuming UI cannot render. Required-arg validation above
            // already covered this for required params; this covers
            // optional-only data tools like PageUpdate.
            let any_non_empty = self.args.keys().any(|name| {
                obj.and_then(|o| o.get(name))
                    .map(|v| match v {
                        Value::Null => false,
                        Value::Object(m) => !m.is_empty(),
                        Value::Array(a) => !a.is_empty(),
                        Value::String(s) => !s.is_empty(),
                        _ => true,
                    })
                    .unwrap_or(false)
            });
            if !any_non_empty {
                anyhow::bail!(
                    "{} call has no payload — provide at least one non-empty argument ({}). {}",
                    self.name,
                    self.args.keys().cloned().collect::<Vec<_>>().join(", "),
                    self.description.trim_end_matches('.'),
                );
            }
            info!("Skill data tool '{}': passthrough", self.name);
            return Ok(ToolResult::CommandOutput {
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
            });
        }

        // Render command template.
        let mut rendered = self.cmd.clone();

        // Replace $SKILL_DIR with the skill's directory path.
        if let Some(skill_dir) = &self.skill_dir {
            rendered = rendered.replace("$SKILL_DIR", &skill_dir.to_string_lossy());
        }

        // Replace {{param}} placeholders with argument values.
        for (name, param) in &self.args {
            let placeholder = format!("{{{{{}}}}}", name);
            let value = obj
                .and_then(|o| o.get(name))
                .or(param.default.as_ref());

            if let Some(val) = value {
                let str_val = match val {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                let escaped = shell_escape_arg(&str_val);
                rendered = rendered.replace(&placeholder, &escaped);
            } else {
                rendered = rendered.replace(&placeholder, "");
            }
        }

        info!("Skill tool '{}' rendered command: {}", self.name, rendered);

        // Execute via sh -c.
        let timeout = Duration::from_millis(self.timeout_ms);
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(&rendered)
            .current_dir(workspace_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let start = Instant::now();
        let mut timed_out = false;
        loop {
            if child.try_wait()?.is_some() {
                break;
            }
            if start.elapsed() >= timeout {
                timed_out = true;
                let _ = child.kill();
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }

        let output = child.wait_with_output()?;
        let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if timed_out {
            if !stderr.is_empty() && !stderr.ends_with('\n') {
                stderr.push('\n');
            }
            stderr.push_str(&format!(
                "linggen: skill tool command timed out after {}ms\n",
                timeout.as_millis()
            ));
        }

        Ok(ToolResult::CommandOutput {
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr,
        })
    }

    /// Convert this skill tool definition to an OpenAI-compatible tool schema.
    pub fn to_oai_schema(&self) -> Value {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for (name, param) in &self.args {
            let mut prop = serde_json::Map::new();
            prop.insert("type".to_string(), Value::String(param.param_type.clone()));
            if !param.description.is_empty() {
                prop.insert("description".to_string(), Value::String(param.description.clone()));
            }
            // OpenAI requires "items" for array types.
            if param.param_type == "array" {
                let items = param.items.clone().unwrap_or_else(|| serde_json::json!({"type": "object"}));
                prop.insert("items".to_string(), items);
            }
            properties.insert(name.clone(), Value::Object(prop));
            if param.required {
                required.push(Value::String(name.clone()));
            }
        }
        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": {
                    "type": "object",
                    "properties": properties,
                    "required": required
                }
            }
        })
    }

    pub fn to_schema_json(&self) -> Value {
        let mut args_map = serde_json::Map::new();
        for (name, param) in &self.args {
            let type_str = if param.required {
                param.param_type.clone()
            } else {
                format!("{}?", param.param_type)
            };
            args_map.insert(name.clone(), serde_json::json!(type_str));
        }

        let mut entry = serde_json::json!({
            "name": self.name,
            "args": args_map,
            "returns": self.returns.as_deref().unwrap_or("string"),
        });

        if !self.description.is_empty() {
            entry["notes"] = serde_json::json!(self.description);
        }

        entry
    }
}

fn shell_escape_arg(s: &str) -> String {
    if s.contains('\'') {
        format!("'{}'", s.replace('\'', "'\\''"))
    } else {
        format!("'{}'", s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn shell_escape_simple() {
        assert_eq!(shell_escape_arg("hello"), "'hello'");
    }

    #[test]
    fn shell_escape_with_single_quote() {
        assert_eq!(shell_escape_arg("it's"), "'it'\\''s'");
    }

    #[test]
    fn to_schema_json_includes_all_fields() {
        let tool = SkillToolDef {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            cmd: "echo {{query}}".to_string(),
            endpoint: None,
            tier: None,
            args: HashMap::from([(
                "query".to_string(),
                SkillParamDef {
                    param_type: "string".to_string(),
                    required: true,
                    default: None,
                    description: "Search query".to_string(),
                    items: None,
                },
            )]),
            returns: Some("stdout text".to_string()),
            timeout_ms: 30000,
            skill_name: None,
            skill_dir: None,
        };

        let schema = tool.to_schema_json();
        assert_eq!(schema["name"], "test_tool");
        assert_eq!(schema["args"]["query"], "string");
        assert_eq!(schema["returns"], "stdout text");
        assert_eq!(schema["notes"], "A test tool");
    }
}

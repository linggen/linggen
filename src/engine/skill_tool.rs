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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillToolDef {
    pub name: String,
    pub description: String,
    pub cmd: String,
    #[serde(default)]
    pub args: HashMap<String, SkillParamDef>,
    #[serde(default)]
    pub returns: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    /// Directory containing the skill file; set at load time.
    #[serde(skip)]
    pub skill_dir: Option<PathBuf>,
}

impl SkillToolDef {
    pub fn execute(&self, args: &Value, workspace_root: &Path) -> Result<ToolResult> {
        let obj = args.as_object();

        // Validate required args.
        for (name, param) in &self.args {
            if param.required {
                let has_arg = obj.map(|o| o.contains_key(name)).unwrap_or(false);
                if !has_arg {
                    anyhow::bail!("missing required argument: {}", name);
                }
            }
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
            args: HashMap::from([(
                "query".to_string(),
                SkillParamDef {
                    param_type: "string".to_string(),
                    required: true,
                    default: None,
                    description: "Search query".to_string(),
                },
            )]),
            returns: Some("stdout text".to_string()),
            timeout_ms: 30000,
            skill_dir: None,
        };

        let schema = tool.to_schema_json();
        assert_eq!(schema["name"], "test_tool");
        assert_eq!(schema["args"]["query"], "string");
        assert_eq!(schema["returns"], "stdout text");
        assert_eq!(schema["notes"], "A test tool");
    }
}

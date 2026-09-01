use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tracing::info;

use super::tools::ToolResult;

/// Recursively detects no-op payloads. Returns true for `null`, empty
/// strings, empty objects/arrays, and — crucially — arrays whose entries
/// are all effectively empty (e.g. `[{}]`) and objects whose fields are
/// all effectively empty (e.g. `{"top_bar": null, "body": null}`).
///
/// Catches the gpt-5.5 pattern of satisfying "non-empty array" by
/// passing `[{}]` while emitting no real content.
fn is_effectively_empty(val: &Value) -> bool {
    match val {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.iter().all(is_effectively_empty),
        Value::Object(m) => m.values().all(is_effectively_empty),
        _ => false, // numbers, bools — concrete content
    }
}

fn default_param_type() -> String {
    "string".to_string()
}

fn default_timeout() -> u64 {
    30000
}

fn default_max_output_bytes() -> usize {
    super::tools::DEFAULT_MAX_TOOL_OUTPUT_BYTES
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
    /// Byte budget for each output stream returned to the model. A skill
    /// whose tool legitimately reports more raises it; one whose script can
    /// spill (a raw page dump, a shape probe) lowers it. Past the budget the
    /// head and tail survive with a marker naming what was dropped.
    #[serde(default = "default_max_output_bytes")]
    pub max_output_bytes: usize,
    /// Name of the skill that declared this tool. Set at skill-load time so
    /// dispatch can resolve the daemon (via `SkillLoader`) without another
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
                        if is_effectively_empty(val) {
                            anyhow::bail!(
                                "{} call has empty '{}' (e.g. {} or {}). {}. Retry with the full payload.",
                                self.name,
                                name,
                                if param.param_type == "object" { "{}" } else { "[]" },
                                if param.param_type == "object" { "all-null fields" } else { "[{}]" },
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
                    .map(|v| !is_effectively_empty(v))
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
            let value = obj.and_then(|o| o.get(name)).or(param.default.as_ref());

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

        // Execute via sh -c. Own the process group so a timeout kills the
        // whole pipeline, not just the shell.
        let timeout = Duration::from_millis(self.timeout_ms);
        let mut child = {
            let mut cmd = Command::new("sh");
            cmd.arg("-c")
                .arg(&rendered)
                .current_dir(workspace_root)
                .env("PATH", crate::util::shell_path())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                cmd.process_group(0);
            }
            cmd.spawn()?
        };

        // Drain both pipes while we wait. The OS pipe buffer is ~64 KB: a
        // command that writes more than that blocks on the write until
        // someone reads, so a poll loop that only watches for exit would see
        // a working command as a timeout.
        let stdout_handle = std::thread::spawn(drain(child.stdout.take()));
        let stderr_handle = std::thread::spawn(drain(child.stderr.take()));

        let start = Instant::now();
        let mut timed_out = false;
        loop {
            if child.try_wait()?.is_some() {
                break;
            }
            if start.elapsed() >= timeout {
                timed_out = true;
                crate::engine::tools::kill_process_group(&child);
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }

        let status = child.wait()?;
        let stdout = stdout_handle.join().unwrap_or_default();
        let mut stderr = stderr_handle.join().unwrap_or_default();
        if timed_out {
            if !stderr.is_empty() && !stderr.ends_with('\n') {
                stderr.push('\n');
            }
            stderr.push_str(&format!(
                "linggen: skill tool command timed out after {}ms\n",
                timeout.as_millis()
            ));
        }

        Ok(ToolResult::command_output(
            status.code(),
            &stdout,
            &stderr,
            self.max_output_bytes,
        ))
    }

    /// Convert this skill tool definition to an OpenAI-compatible tool schema.
    pub fn to_oai_schema(&self) -> Value {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for (name, param) in &self.args {
            let mut prop = serde_json::Map::new();
            prop.insert("type".to_string(), Value::String(param.param_type.clone()));
            if !param.description.is_empty() {
                prop.insert(
                    "description".to_string(),
                    Value::String(param.description.clone()),
                );
            }
            // OpenAI requires "items" for array types.
            if param.param_type == "array" {
                let items = param
                    .items
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({"type": "object"}));
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

/// Reader closure for one of a child's pipes: read it to EOF on its own
/// thread so the child is never blocked writing.
fn drain<R: std::io::Read + Send + 'static>(stream: Option<R>) -> impl FnOnce() -> String + Send {
    move || {
        let mut buf = Vec::new();
        if let Some(mut stream) = stream {
            let _ = stream.read_to_end(&mut buf);
        }
        String::from_utf8_lossy(&buf).to_string()
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
    fn effectively_empty_catches_gpt55_patterns() {
        use serde_json::json;
        // Trivially empty
        assert!(is_effectively_empty(&json!(null)));
        assert!(is_effectively_empty(&json!("")));
        assert!(is_effectively_empty(&json!({})));
        assert!(is_effectively_empty(&json!([])));
        // gpt-5.5 over-fill patterns
        assert!(is_effectively_empty(&json!([{}])));
        assert!(is_effectively_empty(&json!([{}, {}])));
        assert!(is_effectively_empty(
            &json!({"top_bar": null, "body": null})
        ));
        assert!(is_effectively_empty(&json!({"a": {"b": null}})));
        assert!(is_effectively_empty(&json!([{"a": null}, {}])));
        // Concrete content — not empty
        assert!(!is_effectively_empty(&json!("hello")));
        assert!(!is_effectively_empty(&json!(42)));
        assert!(!is_effectively_empty(&json!(true)));
        assert!(!is_effectively_empty(&json!({"key": "val"})));
        assert!(!is_effectively_empty(&json!([{"match": "x"}])));
        assert!(!is_effectively_empty(&json!([{}, {"real": 1}])));
    }

    fn shell_tool(cmd: &str, max_output_bytes: usize) -> SkillToolDef {
        SkillToolDef {
            name: "spill".to_string(),
            description: "Spills a lot".to_string(),
            cmd: cmd.to_string(),
            endpoint: None,
            tier: None,
            args: HashMap::new(),
            returns: None,
            timeout_ms: 30000,
            max_output_bytes,
            skill_name: None,
            skill_dir: None,
        }
    }

    #[test]
    fn a_spilling_shell_tool_comes_back_capped() {
        let tool = shell_tool("head -c 200000 /dev/zero | tr '\\0' 'x'", 64 * 1024);
        let result = tool
            .execute(&serde_json::json!({}), Path::new("."))
            .expect("tool runs");
        let ToolResult::CommandOutput { stdout, .. } = result else {
            panic!("expected CommandOutput");
        };
        assert!(
            stdout.len() < 66 * 1024,
            "200 KB of stdout capped to {} bytes",
            stdout.len()
        );
        assert!(stdout.contains("bytes omitted"), "the model is told");
        assert!(stdout.contains("output was 200000 bytes"));
    }

    #[test]
    fn a_skill_can_declare_its_own_budget() {
        let tool = shell_tool("head -c 200000 /dev/zero | tr '\\0' 'x'", 4096);
        let result = tool
            .execute(&serde_json::json!({}), Path::new("."))
            .expect("tool runs");
        let ToolResult::CommandOutput { stdout, .. } = result else {
            panic!("expected CommandOutput");
        };
        assert!(
            stdout.len() < 5 * 1024,
            "declared budget wins: {}",
            stdout.len()
        );
    }

    #[test]
    fn a_timeout_kills_the_pipeline_and_says_so() {
        let mut tool = shell_tool("sleep 30 | cat", 64 * 1024);
        tool.timeout_ms = 300;
        let start = Instant::now();
        let result = tool
            .execute(&serde_json::json!({}), Path::new("."))
            .expect("tool returns");
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "returned in {:?}, not after the sleep",
            start.elapsed()
        );
        let ToolResult::CommandOutput { stderr, .. } = result else {
            panic!("expected CommandOutput");
        };
        assert!(
            stderr.contains("timed out after 300ms"),
            "stderr: {}",
            stderr
        );
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
            max_output_bytes: default_max_output_bytes(),
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

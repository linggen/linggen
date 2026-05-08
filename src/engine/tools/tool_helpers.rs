use anyhow::Result;
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde_json::Value;
use std::path::{Component, Path};

pub(super) fn build_globset(globs: Option<&[String]>) -> Result<Option<GlobSet>> {
    let Some(globs) = globs else {
        return Ok(None);
    };
    if globs.is_empty() {
        return Ok(None);
    }

    let mut builder = GlobSetBuilder::new();
    for g in globs {
        builder.add(Glob::new(g)?);
    }
    Ok(Some(builder.build()?))
}

/// Expand `~/` prefix to the user's home directory.
pub(crate) fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    path.to_string()
}

pub(super) fn sanitize_rel_path(root: &Path, path: &str) -> Result<String> {
    if path.is_empty() {
        anyhow::bail!("empty path");
    }
    let expanded = expand_tilde(path);
    let raw = Path::new(&expanded);
    let rel_path = if raw.is_absolute() {
        raw.strip_prefix(root)
            .map_err(|_| anyhow::anyhow!("absolute path must be inside workspace root"))?
            .to_path_buf()
    } else {
        raw.to_path_buf()
    };

    if rel_path.as_os_str().is_empty() {
        anyhow::bail!("empty path");
    }
    if rel_path
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        anyhow::bail!("path traversal not allowed");
    }
    if rel_path
        .components()
        .any(|c| matches!(c, Component::RootDir | Component::Prefix(_)))
    {
        anyhow::bail!("path must resolve inside workspace root");
    }

    Ok(rel_path.to_string_lossy().to_string())
}

pub(super) fn to_rel_string(root: &Path, path: &Path) -> Result<String> {
    let rel = path.strip_prefix(root)?;
    Ok(rel.to_string_lossy().to_string())
}

pub(crate) fn summarize_tool_args(tool: &str, args: &Value) -> String {
    let mut safe_args = args.clone();
    if let Some(obj) = safe_args.as_object_mut() {
        match tool {
            "Write" => {
                if let Some(content) = obj.get("content").and_then(|v| v.as_str()) {
                    let byte_len = content.len();
                    let line_count = content.lines().count();
                    obj.insert(
                        "content".to_string(),
                        serde_json::json!(format!(
                            "<omitted:{} bytes, {} lines>",
                            byte_len, line_count
                        )),
                    );
                }
            }
            "Edit" => {
                for key in ["old_string", "new_string", "old", "new", "old_text", "new_text", "oldText", "newText", "search", "replace", "from", "to"] {
                    if let Some(content) = obj.get(key).and_then(|v| v.as_str()) {
                        let byte_len = content.len();
                        let line_count = content.lines().count();
                        obj.insert(
                            key.to_string(),
                            serde_json::json!(format!(
                                "<omitted:{} bytes, {} lines>",
                                byte_len, line_count
                            )),
                        );
                    }
                }
            }
            "Bash" => {
                if let Some(cmd) = obj.get("cmd").and_then(|v| v.as_str()) {
                    let preview = if cmd.len() > 160 {
                        // Find a char boundary at or before 160 to avoid UTF-8 panic.
                        let end = cmd
                            .char_indices()
                            .map(|(i, _)| i)
                            .take_while(|&i| i <= 160)
                            .last()
                            .unwrap_or(0);
                        format!("{}... (truncated, {} chars)", &cmd[..end], cmd.len())
                    } else {
                        cmd.to_string()
                    };
                    obj.insert("cmd".to_string(), serde_json::json!(preview));
                }
            }
            _ => {}
        }
    }
    safe_args.to_string()
}

pub(crate) fn normalize_tool_args(tool: &str, args: Value) -> Value {
    let mut normalized = args;
    if let Some(obj) = normalized.as_object_mut() {
        if matches!(tool, "Bash") && !obj.contains_key("cmd") {
            if let Some(command) = obj.get("command").cloned() {
                obj.insert("cmd".to_string(), command);
            }
        }

        if matches!(tool, "Read" | "Write" | "Edit") && !obj.contains_key("path") {
            if let Some(fp) = obj.get("filepath").cloned() {
                obj.insert("path".to_string(), fp);
            } else if let Some(file) = obj.get("file").cloned() {
                obj.insert("path".to_string(), file);
            }
        }

        if matches!(tool, "Edit") {
            if !obj.contains_key("old_string") {
                if let Some(v) = obj.get("old").cloned() {
                    obj.insert("old_string".to_string(), v);
                } else if let Some(v) = obj.get("old_text").cloned() {
                    obj.insert("old_string".to_string(), v);
                } else if let Some(v) = obj.get("oldText").cloned() {
                    obj.insert("old_string".to_string(), v);
                } else if let Some(v) = obj.get("search").cloned() {
                    obj.insert("old_string".to_string(), v);
                } else if let Some(v) = obj.get("from").cloned() {
                    obj.insert("old_string".to_string(), v);
                }
            }
            if !obj.contains_key("new_string") {
                if let Some(v) = obj.get("new").cloned() {
                    obj.insert("new_string".to_string(), v);
                } else if let Some(v) = obj.get("new_text").cloned() {
                    obj.insert("new_string".to_string(), v);
                } else if let Some(v) = obj.get("newText").cloned() {
                    obj.insert("new_string".to_string(), v);
                } else if let Some(v) = obj.get("replace").cloned() {
                    obj.insert("new_string".to_string(), v);
                } else if let Some(v) = obj.get("to").cloned() {
                    obj.insert("new_string".to_string(), v);
                }
            }
            if !obj.contains_key("replace_all") {
                if let Some(v) = obj.get("all").cloned() {
                    obj.insert("replace_all".to_string(), v);
                }
            }
        }

        // Normalize query aliases for Grep. Note: "path" is intentionally excluded
        // because it's the directory/file scope argument, not a search pattern.
        if matches!(tool, "Grep") && !obj.contains_key("query") {
            if let Some(pat) = obj.get("pattern").cloned() {
                obj.insert("query".to_string(), pat);
            } else if let Some(fp) = obj.get("filepath").cloned() {
                obj.insert("query".to_string(), fp);
            } else if let Some(file) = obj.get("file").cloned() {
                obj.insert("query".to_string(), file);
            }
        }

        // Normalize "pattern" → "globs" for Glob tool. Models often emit
        // {"pattern":"**/*.rs"} instead of {"globs":["**/*.rs"]}.
        if matches!(tool, "Glob") && !obj.contains_key("globs") {
            if let Some(pat) = obj
                .get("pattern")
                .or_else(|| obj.get("glob"))
                .cloned()
            {
                if let Some(s) = pat.as_str() {
                    obj.insert("globs".to_string(), serde_json::json!([s]));
                } else if pat.is_array() {
                    obj.insert("globs".to_string(), pat);
                }
            }
        }

        if matches!(tool, "Grep" | "Glob")
            && obj.get("globs").map(|v| v.is_string()).unwrap_or(false)
        {
            if let Some(glob) = obj.get("globs").and_then(|v| v.as_str()) {
                obj.insert("globs".to_string(), serde_json::json!([glob]));
            }
        }
    }
    normalized
}

/// Resolve a tool name (canonical or alias) to its canonical form.
///
/// Built-in tools delegate to the [`super::builtin::lookup`] registry —
/// adding a tool there auto-registers its name and aliases here. Plan-mode
/// tools (`EnterPlanMode`, `ExitPlanMode`, `UpdatePlan`) aren't real
/// `Tool` impls (they're parsed as `ModelAction`s in `actions.rs`) so
/// they keep an explicit branch.
pub fn canonical_tool_name(tool: &str) -> Option<&'static str> {
    if let Some(t) = super::builtin::lookup(tool) {
        return Some(t.name());
    }
    Some(match tool {
        "ExitPlanMode" | "exit_plan_mode" => "ExitPlanMode",
        "EnterPlanMode" | "enter_plan_mode" => "EnterPlanMode",
        "UpdatePlan" | "update_plan" => "UpdatePlan",
        _ => return None,
    })
}

/// Full short-form schema list for the system-prompt JSON-action embedding.
///
/// Combines built-in tools (sourced from the [`super::builtin`] registry —
/// each `Tool::legacy_schema_entry()` contributes one entry) with the
/// plan-mode tools, which are parsed as `ModelAction`s rather than real
/// `Tool` impls.
pub(crate) fn full_tool_schema_entries() -> Vec<Value> {
    let mut out = super::builtin::model_facing_legacy_entries();
    out.push(serde_json::json!({
        "name": "ExitPlanMode",
        "args": {"plan_text": "string", "items": "[{id: string, title: string, status: string}]?"},
        "returns": "success",
        "notes": "Submit your plan for user approval. Include the full detailed plan in plan_text. Optionally include items as a structured task list for progress tracking (all pending). If items is omitted, the system auto-extracts steps from your plan text."
    }));
    out.push(serde_json::json!({
        "name": "EnterPlanMode",
        "args": {"reason": "string?"},
        "returns": "success",
        "notes": "Enter plan mode to research and produce a detailed implementation plan. Restricts you to read-only tools until you call ExitPlanMode."
    }));
    out.push(serde_json::json!({
        "name": "UpdatePlan",
        "args": {"plan_text": "string?", "items": "[{id: string, title: string, status: string}]?"},
        "returns": "success",
        "notes": "Track execution progress during plan execution (after approval). Update item status: pending → in_progress → completed. Do NOT call this during planning — use ExitPlanMode instead."
    }));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_glob_pattern_to_globs() {
        let args = serde_json::json!({"pattern": "**/SKILL.md"});
        let result = normalize_tool_args("Glob", args);
        assert_eq!(result["globs"], serde_json::json!(["**/SKILL.md"]));
    }

    #[test]
    fn normalize_glob_single_string_to_array() {
        let args = serde_json::json!({"globs": "**/*.rs"});
        let result = normalize_tool_args("Glob", args);
        assert_eq!(result["globs"], serde_json::json!(["**/*.rs"]));
    }

    #[test]
    fn normalize_glob_already_array_untouched() {
        let args = serde_json::json!({"globs": ["**/*.rs", "**/*.toml"]});
        let result = normalize_tool_args("Glob", args);
        assert_eq!(result["globs"], serde_json::json!(["**/*.rs", "**/*.toml"]));
    }

    #[test]
    fn normalize_grep_pattern_to_query() {
        let args = serde_json::json!({"pattern": "fn main"});
        let result = normalize_tool_args("Grep", args);
        assert_eq!(result["query"], "fn main");
    }

    #[test]
    fn normalize_glob_pattern_does_not_override_globs() {
        // If both "globs" and "pattern" are present, "globs" wins.
        let args = serde_json::json!({"globs": ["**/*.rs"], "pattern": "**/SKILL.md"});
        let result = normalize_tool_args("Glob", args);
        assert_eq!(result["globs"], serde_json::json!(["**/*.rs"]));
    }
}

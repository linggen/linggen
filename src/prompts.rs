//! Prompt template store.
//!
//! Loads prompt templates from structured TOML files under `prompts/` at compile
//! time, falling back to compiled-in defaults.  User overrides from
//! `~/.linggen/prompts/` (both `.toml` and legacy `.md`) are loaded at runtime.
//! Templates use `{variable}` placeholders substituted via [`PromptStore::render`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Well-known prompt keys (dot-notation: "{file_stem}.{table_name}")
// ---------------------------------------------------------------------------

pub mod keys {
    // -- system-prompt.* --------------------------------------------------
    pub const SYSTEM_FALLBACK_IDENTITY: &str = "system-prompt.fallback_identity";
    pub const SYSTEM_SKILLS_HEADER: &str = "system-prompt.skills_header";
    pub const SYSTEM_SKILL_ENTRY: &str = "system-prompt.skill_entry";
    pub const SYSTEM_ACTIVE_SKILL_FRAME: &str = "system-prompt.active_skill_frame";
    pub const SYSTEM_PROJECT_INSTRUCTIONS_HEADER: &str =
        "system-prompt.project_instructions_header";
    pub const SYSTEM_PROJECT_INSTRUCTIONS_ENTRY: &str = "system-prompt.project_instructions_entry";
    pub const SYSTEM_PROJECT_INSTRUCTIONS_FOOTER: &str =
        "system-prompt.project_instructions_footer";
    pub const SYSTEM_ENVIRONMENT_BLOCK: &str = "system-prompt.environment_block";
    pub const SYSTEM_MEMORY_BLOCK: &str = "system-prompt.memory_block";
    pub const SYSTEM_MEMORY_BLOCK_EMPTY: &str = "system-prompt.memory_block_empty";

    // -- system-reminder.* ------------------------------------------------
    pub const NUDGE_INVALID_JSON: &str = "system-reminder.nudge_invalid_json";
    pub const NUDGE_REPETITION: &str = "system-reminder.nudge_repetition";
    pub const NUDGE_REDUNDANT_TOOL: &str = "system-reminder.nudge_redundant_tool";
    pub const NUDGE_EMPTY_SEARCH: &str = "system-reminder.nudge_empty_search";
    pub const TOOL_NOT_ALLOWED: &str = "system-reminder.tool_not_allowed";
    pub const WRITE_SAFETY_BLOCKED: &str = "system-reminder.write_safety_blocked";
    pub const TOOL_EXEC_FAILED: &str = "system-reminder.tool_exec_failed";
    pub const PERMISSION_DENIED: &str = "system-reminder.permission_denied";
    pub const PERMISSION_TIMEOUT: &str = "system-reminder.permission_timeout";
    pub const PATCH_NOT_ALLOWED: &str = "system-reminder.patch_not_allowed";
    pub const PATCH_VALIDATION_FAILED: &str = "system-reminder.patch_validation_failed";
    pub const DELEGATION_BLOCKED: &str = "system-reminder.delegation_blocked";
    pub const DELEGATION_VALIDATION_FAILED: &str = "system-reminder.delegation_validation_failed";
    pub const DELEGATION_FAILED: &str = "system-reminder.delegation_failed";
    pub const EXIT_PLAN_MODE_OUTSIDE_PLAN: &str = "system-reminder.exit_plan_mode_outside_plan";
    pub const INVALID_TASK_ARGS: &str = "system-reminder.invalid_task_args";

    // -- tool-result.* ----------------------------------------------------
    pub const FILE_NOT_FOUND_MEMORY: &str = "tool-result.file_not_found_memory";
    pub const FILE_NOT_FOUND_EXHAUSTED: &str = "tool-result.file_not_found_exhausted";
    pub const READ_IS_DIRECTORY: &str = "tool-result.read_is_directory";
    pub const SMART_SEARCH_REDIRECT: &str = "tool-result.smart_search_redirect";
    pub const SMART_SEARCH_REDIRECT_MULTI: &str = "tool-result.smart_search_redirect_multi";
    pub const ASKUSER_SUBAGENT_BLOCKED: &str = "tool-result.askuser_subagent_blocked";
    pub const ASKUSER_CLI_BLOCKED: &str = "tool-result.askuser_cli_blocked";
    pub const ASKUSER_CANCELLED: &str = "tool-result.askuser_cancelled";
    pub const ASKUSER_TIMEOUT: &str = "tool-result.askuser_timeout";
    pub const PLAN_SUBMITTED: &str = "tool-result.plan_submitted";
    pub const PLAN_UPDATED: &str = "tool-result.plan_updated";
    pub const DONE_DEFAULT: &str = "tool-result.done_default";
    pub const OBSERVATION_WRAPPER: &str = "tool-result.observation_wrapper";
    pub const COMPACTION_SUMMARY: &str = "tool-result.compaction_summary";

    // -- bailout.* --------------------------------------------------------
    pub const BAILOUT_LOOP_LIMIT: &str = "bailout.loop_limit";
    pub const BAILOUT_MALFORMED_OUTPUT: &str = "bailout.malformed_output";
    pub const BAILOUT_REPETITION_LOOP: &str = "bailout.repetition_loop";

    // -- Single-table files (legacy compat) -------------------------------
    pub const RESPONSE_FORMAT: &str = "response-format.default";
    pub const RESPONSE_FORMAT_NATIVE: &str = "response-format.native";
    pub const PLAN_MODE: &str = "plan-mode.default";
    pub const PLAN_EXECUTE: &str = "plan-execute.default";
    pub const TASK_BOOTSTRAP: &str = "task-bootstrap.default";
}

// Legacy re-exports so existing `crate::prompts::RESPONSE_FORMAT` etc. still work.
pub use keys::{
    NUDGE_INVALID_JSON, NUDGE_REDUNDANT_TOOL, NUDGE_REPETITION, PLAN_EXECUTE, PLAN_MODE,
    RESPONSE_FORMAT, RESPONSE_FORMAT_NATIVE, TASK_BOOTSTRAP,
};

// ---------------------------------------------------------------------------
// Embedded TOML defaults (compile-time)
// ---------------------------------------------------------------------------

const TOML_DEFAULTS: &[(&str, &str)] = &[
    (
        "system-prompt",
        include_str!("../prompts/system-prompt.toml"),
    ),
    (
        "system-reminder",
        include_str!("../prompts/system-reminder.toml"),
    ),
    ("tool-result", include_str!("../prompts/tool-result.toml")),
    ("bailout", include_str!("../prompts/bailout.toml")),
    (
        "response-format",
        include_str!("../prompts/response-format.toml"),
    ),
    ("plan-mode", include_str!("../prompts/plan-mode.toml")),
    (
        "plan-execute",
        include_str!("../prompts/plan-execute.toml"),
    ),
    (
        "task-bootstrap",
        include_str!("../prompts/task-bootstrap.toml"),
    ),
];

// ---------------------------------------------------------------------------
// PromptStore
// ---------------------------------------------------------------------------

/// Runtime prompt template store.
///
/// On construction it parses embedded TOML defaults, then overlays any files
/// found in `override_dir` (typically `~/.linggen/prompts/`).  Supports both
/// `.toml` (grouped tables) and legacy `.md` (single-key) overrides.
pub struct PromptStore {
    prompts: HashMap<String, String>,
}

impl PromptStore {
    /// Create a store seeded with compiled-in defaults, optionally overlaid
    /// with files from `override_dir`.
    pub fn load(override_dir: Option<&Path>) -> Self {
        let mut prompts: HashMap<String, String> = HashMap::new();

        for (stem, src) in TOML_DEFAULTS {
            Self::load_toml_into(&mut prompts, stem, src);
        }

        if let Some(dir) = override_dir {
            Self::overlay_from_dir(&mut prompts, dir);
        }

        Self { prompts }
    }

    /// Get a raw template by key.  Returns `None` for unknown keys.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.prompts.get(key).map(|s| s.as_str())
    }

    /// Render a template, replacing every `{name}` with the corresponding
    /// value from `vars`.  Unknown keys in the template are left as-is.
    pub fn render(&self, key: &str, vars: &[(&str, &str)]) -> Option<String> {
        self.get(key).map(|tpl| Self::substitute(tpl, vars))
    }

    /// Render a template with fallback: returns `[missing prompt: key]` on miss.
    pub fn render_or_fallback(&self, key: &str, vars: &[(&str, &str)]) -> String {
        self.render(key, vars)
            .unwrap_or_else(|| format!("[missing prompt: {}]", key))
    }

    /// Substitute `{name}` placeholders in `tpl`.
    pub fn substitute(tpl: &str, vars: &[(&str, &str)]) -> String {
        let mut out = tpl.to_string();
        for (name, value) in vars {
            out = out.replace(&format!("{{{}}}", name), value);
        }
        out
    }

    /// Default prompts dir: `~/.linggen/prompts/`.
    pub fn default_override_dir() -> PathBuf {
        crate::paths::linggen_home().join("prompts")
    }

    // -- private -------------------------------------------------------------

    /// Parse a TOML source and insert each table's `text` field as
    /// `"{stem}.{table_name}"` into `prompts`.
    fn load_toml_into(prompts: &mut HashMap<String, String>, stem: &str, src: &str) {
        let table: toml::Table = match src.parse() {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("Failed to parse TOML prompt '{}': {}", stem, e);
                return;
            }
        };
        for (key, value) in &table {
            if let Some(inner) = value.as_table() {
                if let Some(text) = inner.get("text").and_then(|v| v.as_str()) {
                    prompts.insert(format!("{}.{}", stem, key), text.to_string());
                }
            }
        }
    }

    /// Read files in `dir` and insert/overwrite matching keys.
    /// Supports `.toml` (grouped tables) and legacy `.md` (single key = stem).
    fn overlay_from_dir(prompts: &mut HashMap<String, String>, dir: &Path) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return, // dir doesn't exist — that's fine
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str());
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            match ext {
                Some("toml") => {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        Self::load_toml_into(prompts, stem, &content);
                    }
                }
                Some("md") => {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        prompts.insert(stem.to_string(), content);
                    }
                }
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_defaults_load() {
        let store = PromptStore::load(None);
        // Legacy keys (now routed through TOML)
        assert!(store.get(RESPONSE_FORMAT).is_some());
        assert!(store.get(PLAN_MODE).is_some());
        assert!(store.get(TASK_BOOTSTRAP).is_some());
        assert!(store.get(NUDGE_INVALID_JSON).is_some());
        // New keys
        assert!(store.get(keys::SYSTEM_FALLBACK_IDENTITY).is_some());
        assert!(store.get(keys::BAILOUT_LOOP_LIMIT).is_some());
        assert!(store.get(keys::OBSERVATION_WRAPPER).is_some());
    }

    #[test]
    fn render_substitutes_vars() {
        let store = PromptStore::load(None);
        let rendered = store
            .render(NUDGE_REDUNDANT_TOOL, &[("tool", "Read")])
            .unwrap();
        assert!(rendered.contains("'Read'"));
        assert!(!rendered.contains("{tool}"));
    }

    #[test]
    fn render_preserves_unknown_vars() {
        let out = PromptStore::substitute("hello {name}, {unknown} world", &[("name", "alice")]);
        assert_eq!(out, "hello alice, {unknown} world");
    }

    #[test]
    fn render_or_fallback_missing_key() {
        let store = PromptStore::load(None);
        let out = store.render_or_fallback("nonexistent.key", &[]);
        assert_eq!(out, "[missing prompt: nonexistent.key]");
    }

    #[test]
    fn overlay_from_tempdir_toml() {
        let tmp = std::env::temp_dir().join("linggen_prompt_toml_test");
        let _ = std::fs::create_dir_all(&tmp);
        std::fs::write(
            tmp.join("system-reminder.toml"),
            "[nudge_redundant_tool]\ntext = \"custom nudge for {tool}\"",
        )
        .unwrap();

        let store = PromptStore::load(Some(&tmp));
        let rendered = store
            .render(keys::NUDGE_REDUNDANT_TOOL, &[("tool", "Glob")])
            .unwrap();
        assert_eq!(rendered, "custom nudge for Glob");

        // cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn overlay_from_tempdir_md_legacy() {
        let tmp = std::env::temp_dir().join("linggen_prompt_md_test");
        let _ = std::fs::create_dir_all(&tmp);
        std::fs::write(tmp.join("custom-key.md"), "legacy content").unwrap();

        let store = PromptStore::load(Some(&tmp));
        assert_eq!(store.get("custom-key").unwrap(), "legacy content");

        // cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn all_keys_resolve() {
        let store = PromptStore::load(None);
        let all_keys = [
            keys::SYSTEM_FALLBACK_IDENTITY,
            keys::SYSTEM_SKILLS_HEADER,
            keys::SYSTEM_SKILL_ENTRY,
            keys::SYSTEM_ACTIVE_SKILL_FRAME,
            keys::SYSTEM_PROJECT_INSTRUCTIONS_HEADER,
            keys::SYSTEM_PROJECT_INSTRUCTIONS_ENTRY,
            keys::SYSTEM_PROJECT_INSTRUCTIONS_FOOTER,
            keys::SYSTEM_ENVIRONMENT_BLOCK,
            keys::SYSTEM_MEMORY_BLOCK,
            keys::SYSTEM_MEMORY_BLOCK_EMPTY,
            keys::NUDGE_INVALID_JSON,
            keys::NUDGE_REPETITION,
            keys::NUDGE_REDUNDANT_TOOL,
            keys::NUDGE_EMPTY_SEARCH,
            keys::TOOL_NOT_ALLOWED,
            keys::WRITE_SAFETY_BLOCKED,
            keys::TOOL_EXEC_FAILED,
            keys::PERMISSION_DENIED,
            keys::PERMISSION_TIMEOUT,
            keys::PATCH_NOT_ALLOWED,
            keys::PATCH_VALIDATION_FAILED,
            keys::DELEGATION_BLOCKED,
            keys::DELEGATION_VALIDATION_FAILED,
            keys::DELEGATION_FAILED,
            keys::EXIT_PLAN_MODE_OUTSIDE_PLAN,
            keys::INVALID_TASK_ARGS,
            keys::FILE_NOT_FOUND_MEMORY,
            keys::FILE_NOT_FOUND_EXHAUSTED,
            keys::READ_IS_DIRECTORY,
            keys::SMART_SEARCH_REDIRECT,
            keys::SMART_SEARCH_REDIRECT_MULTI,
            keys::ASKUSER_SUBAGENT_BLOCKED,
            keys::ASKUSER_CLI_BLOCKED,
            keys::ASKUSER_CANCELLED,
            keys::ASKUSER_TIMEOUT,
            keys::PLAN_SUBMITTED,
            keys::PLAN_UPDATED,
            keys::DONE_DEFAULT,
            keys::OBSERVATION_WRAPPER,
            keys::COMPACTION_SUMMARY,
            keys::BAILOUT_LOOP_LIMIT,
            keys::BAILOUT_MALFORMED_OUTPUT,
            keys::BAILOUT_REPETITION_LOOP,
            keys::RESPONSE_FORMAT,
            keys::RESPONSE_FORMAT_NATIVE,
            keys::PLAN_MODE,
            keys::PLAN_EXECUTE,
            keys::TASK_BOOTSTRAP,
        ];
        for key in all_keys {
            assert!(
                store.get(key).is_some(),
                "Key '{}' not found in prompt store",
                key
            );
        }
    }
}

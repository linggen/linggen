//! Memory dispatch + mid-session self-review nudge.
//!
//! Built-in core memory (identity + style) lives in `core_memory.rs`. Skill
//! memory (facts, activity, semantic retrieval) routes through `dispatch`
//! below: the model calls `Memory.search`, `Memory.add`, etc., and this
//! module forwards to whichever installed skill advertises
//! `provides: [memory]` by shelling out to its binary.
//!
//! Binary-invocation contract (locked — matches
//! `linggen-memory/doc/tech-spec.md` v0.1):
//!
//! - **Binary:** `$SKILL_DIR/bin/ling-mem`, falling back to `ling-mem` on
//!   `$PATH` when the skill directory has no binary (for dev installs).
//! - **Invocation:** `ling-mem <method> [positional] [flags]` with
//!   `LINGGEN_DATA_DIR` exported so the store picks the right per-user root.
//! - **Response:** NDJSON on stdout (one JSON object per line for list
//!   results, single JSON object for single-row results) on exit 0; JSON
//!   error `{"error": "...", "code": "..."}` on stderr with non-zero exit.
//! - **Timeout:** 5 seconds per call. Long-running ops are the skill's
//!   `serve` daemon's job — out of scope for sync dispatch.

use crate::ollama::ChatMessage;
use crate::skills::{Skill, SkillManager};
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;

// ── Memory skill dispatch ────────────────────────────────────────────────────

/// Canonical `Memory.*` method names. Kept in sync with
/// `engine::tools::tool_helpers::MEMORY_TOOL_NAMES` and with
/// `linggen-memory/doc/tech-spec.md` v0.1 subcommands.
pub(crate) const MEMORY_METHODS: &[&str] = &[
    "add", "get", "search", "list", "update", "delete", "forget",
];

const DISPATCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Resolve `Memory.<method>` and invoke it on the active memory provider.
///
/// Returns an `Err` (surfaced verbatim to the model as a tool error) when:
/// no provider is installed, the provider's binary can't be located, the
/// subprocess fails, or the 5-second timeout fires. On success, returns
/// the provider's stdout parsed as JSON — an object for single-row
/// responses, an array for NDJSON list responses.
pub(crate) async fn dispatch(
    skills: &SkillManager,
    method: &str,
    args: Value,
) -> Result<Value> {
    if !MEMORY_METHODS.contains(&method) {
        return Err(anyhow!("unknown Memory method: {method}"));
    }

    let provider = skills.active_provider("memory").await.ok_or_else(|| {
        anyhow!(
            "No memory provider is installed. Install a skill that declares \
             `provides: [memory]` (e.g. linggen-memory) from the skill \
             marketplace, then retry."
        )
    })?;

    let binary = resolve_provider_binary(&provider)?;
    let cli_args = translate_args(method, &args)
        .with_context(|| format!("translating Memory.{method} args to ling-mem flags"))?;

    let output = tokio::time::timeout(DISPATCH_TIMEOUT, async {
        tokio::process::Command::new(&binary)
            .arg(method)
            .args(&cli_args)
            .env("LINGGEN_DATA_DIR", crate::paths::linggen_home())
            .kill_on_drop(true)
            .output()
            .await
    })
    .await
    .map_err(|_| {
        anyhow!(
            "Memory provider '{}' timed out after {}s. If this query is \
             genuinely expensive, use the skill's daemon mode instead.",
            provider.name,
            DISPATCH_TIMEOUT.as_secs(),
        )
    })?
    .with_context(|| format!("invoking memory provider at {}", binary.display()))?;

    if !output.status.success() {
        return Err(provider_error_from_stderr(&provider.name, &output));
    }
    parse_stdout(&output.stdout)
}

/// Locate the provider's binary. Looks for `$SKILL_DIR/bin/ling-mem`
/// first — matches what the skill's `install.sh` places on disk — then
/// falls back to a bare `ling-mem` lookup so dev installs (`cargo install`)
/// work without manual symlinks. Returns an error rather than `Option` so
/// callers surface a clean "provider installed but binary missing"
/// message to the user.
fn resolve_provider_binary(skill: &Skill) -> Result<PathBuf> {
    if let Some(dir) = &skill.skill_dir {
        let candidate = dir.join("bin").join("ling-mem");
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    // Fallback: resolve `ling-mem` against $PATH. `Command::new` will do
    // this at spawn time; we return the name as-is so the spawn attempt
    // produces a normal "program not found" error the user can act on.
    Ok(PathBuf::from("ling-mem"))
}

/// Translate a JSON args object into `ling-mem` CLI args per method.
/// Returns the positional + flag arguments ready for `Command::args`.
fn translate_args(method: &str, args: &Value) -> Result<Vec<String>> {
    let obj = args.as_object().cloned().unwrap_or_default();
    let mut out = Vec::new();

    let flag_string = |out: &mut Vec<String>, key: &str, flag: &str| {
        if let Some(v) = obj.get(key).and_then(|v| v.as_str()) {
            out.push(flag.to_string());
            out.push(v.to_string());
        }
    };
    let flag_int = |out: &mut Vec<String>, key: &str, flag: &str| {
        if let Some(v) = obj.get(key).and_then(|v| v.as_i64()) {
            out.push(flag.to_string());
            out.push(v.to_string());
        }
    };
    let flag_repeated = |out: &mut Vec<String>, key: &str, flag: &str| {
        if let Some(arr) = obj.get(key).and_then(|v| v.as_array()) {
            for item in arr.iter().filter_map(|v| v.as_str()) {
                out.push(flag.to_string());
                out.push(item.to_string());
            }
        }
    };

    match method {
        "add" => {
            let content = obj
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("Memory.add requires `content`"))?;
            out.push(content.to_string());
            flag_string(&mut out, "type", "--type");
            flag_repeated(&mut out, "contexts", "--context");
            flag_repeated(&mut out, "tags", "--tag");
            flag_string(&mut out, "from", "--from");
            flag_string(&mut out, "outcome", "--outcome");
            flag_string(&mut out, "cwd", "--cwd");
            flag_string(&mut out, "occurred_at", "--occurred-at");
            flag_string(&mut out, "source_session", "--source-session");
        }
        "get" => {
            let id = obj
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("Memory.get requires `id`"))?;
            out.push(id.to_string());
        }
        "search" => {
            let query = obj
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("Memory.search requires `query`"))?;
            out.push(query.to_string());
            flag_repeated(&mut out, "contexts", "--context");
            flag_string(&mut out, "type", "--type");
            flag_string(&mut out, "from", "--from");
            flag_string(&mut out, "outcome", "--outcome");
            flag_string(&mut out, "since", "--since");
            flag_int(&mut out, "limit", "--limit");
        }
        "list" => {
            flag_repeated(&mut out, "contexts", "--context");
            flag_string(&mut out, "type", "--type");
            flag_string(&mut out, "from", "--from");
            flag_string(&mut out, "outcome", "--outcome");
            flag_string(&mut out, "since", "--since");
            flag_int(&mut out, "limit", "--limit");
            flag_string(&mut out, "sort", "--sort");
        }
        "update" => {
            let id = obj
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("Memory.update requires `id`"))?;
            out.push(id.to_string());
            flag_string(&mut out, "content", "--content");
            flag_repeated(&mut out, "add_contexts", "--add-context");
            flag_repeated(&mut out, "remove_contexts", "--remove-context");
            flag_repeated(&mut out, "add_tags", "--add-tag");
            flag_repeated(&mut out, "remove_tags", "--remove-tag");
            flag_string(&mut out, "type", "--type");
            flag_string(&mut out, "outcome", "--outcome");
        }
        "delete" => {
            let id = obj
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("Memory.delete requires `id`"))?;
            out.push(id.to_string());
            // Linggen's permission layer already secured user approval;
            // the CLI's own confirmation is redundant noise here.
            out.push("--yes".to_string());
        }
        "forget" => {
            flag_repeated(&mut out, "contexts", "--context");
            flag_string(&mut out, "type", "--type");
            flag_string(&mut out, "older_than", "--older-than");
            out.push("--yes".to_string());
        }
        _ => {
            return Err(anyhow!("unknown Memory method: {method}"));
        }
    }

    Ok(out)
}

/// Build an anyhow error from the provider's stderr. Prefers the
/// structured `{"error": "...", "code": "..."}` shape when the provider
/// emits valid JSON; falls back to the raw string otherwise.
fn provider_error_from_stderr(provider_name: &str, output: &std::process::Output) -> anyhow::Error {
    let stderr = String::from_utf8_lossy(&output.stderr);
    if let Ok(parsed) = serde_json::from_str::<Value>(&stderr) {
        if let Some(obj) = parsed.as_object() {
            let msg = obj.get("error").and_then(|e| e.as_str()).unwrap_or("");
            let code = obj.get("code").and_then(|c| c.as_str());
            if !msg.is_empty() {
                return match code {
                    Some(c) => anyhow!("memory provider '{provider_name}' error [{c}]: {msg}"),
                    None => anyhow!("memory provider '{provider_name}' error: {msg}"),
                };
            }
        }
    }
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        anyhow!(
            "memory provider '{provider_name}' exited with status {} (no stderr)",
            output.status
        )
    } else {
        anyhow!("memory provider '{provider_name}' error: {trimmed}")
    }
}

/// Parse ling-mem's stdout:
/// - empty → `null`
/// - single JSON object or array → that value
/// - multiple lines of JSON → JSON array of the parsed lines (NDJSON)
fn parse_stdout(stdout: &[u8]) -> Result<Value> {
    let text = std::str::from_utf8(stdout).context("provider stdout is not UTF-8")?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Value::Null);
    }

    let lines: Vec<&str> = trimmed.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    if lines.len() <= 1 {
        return serde_json::from_str(trimmed)
            .with_context(|| format!("parsing provider stdout as JSON: {trimmed}"));
    }

    let mut rows = Vec::with_capacity(lines.len());
    for line in lines {
        let v: Value = serde_json::from_str(line)
            .with_context(|| format!("parsing NDJSON line from provider: {line}"))?;
        rows.push(v);
    }
    Ok(json!(rows))
}

// ── Mid-session self-review nudge ────────────────────────────────────────────

/// Returns `true` when the mid-session memory-check nudge should fire for
/// this turn. Fires every `interval` user messages; `interval == 0` disables.
pub(crate) fn should_fire_nudge(chat_history: &[ChatMessage], interval: usize) -> bool {
    if interval == 0 {
        return false;
    }
    let user_count = chat_history.iter().filter(|m| m.role == "user").count();
    user_count > 0 && user_count % interval == 0
}

/// The synthetic user message that nudges the model to check whether the
/// recent exchange produced anything worth saving to, or contradicts
/// something already in, memory.
pub(crate) fn nudge_message() -> ChatMessage {
    ChatMessage::new(
        "user",
        "[MEMORY CHECK — hidden reminder, not from the user] \
         Did the last few exchanges produce anything durable — an \
         identity fact, a cross-project preference, or a scoped fact \
         worth saving? Or did the user contradict something already in \
         memory? If yes, act now: Edit `~/.linggen/core/identity.md` or \
         `style.md` for universals (tiny, high-bar); call `Memory.add` \
         (or `Memory.update`) for scoped facts when a memory provider \
         is installed. Keep project-specific rules out of core. If \
         nothing durable, reply briefly with `(no memory changes)` and \
         continue with the user's current request."
            .to_string(),
    )
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::SkillSource;

    fn bare_skill(name: &str, provides: Option<Vec<String>>, skill_dir: Option<PathBuf>) -> Skill {
        Skill {
            name: name.to_string(),
            description: "test".to_string(),
            content: String::new(),
            source: SkillSource::Global,
            tool_defs: Vec::new(),
            argument_hint: None,
            disable_model_invocation: false,
            user_invocable: true,
            allowed_tools: None,
            model: None,
            context: None,
            agent: None,
            trigger: None,
            app: None,
            permission: None,
            install: None,
            provides,
            skill_dir,
        }
    }

    #[tokio::test]
    async fn dispatch_with_no_provider_errors_with_install_hint() {
        let skills = SkillManager::new();
        let err = dispatch(&skills, "search", json!({"query": "x"}))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("provides: [memory]"),
            "error should hint at the provides contract, got: {err}"
        );
    }

    #[tokio::test]
    async fn dispatch_rejects_unknown_method() {
        let skills = SkillManager::new();
        let err = dispatch(&skills, "nonsense", json!({}))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown Memory method"), "got: {err}");
    }

    #[test]
    fn translate_add_emits_content_and_flags() {
        let args = json!({
            "content": "Likes dark mode",
            "type": "preference",
            "contexts": ["code/linggen", "ui"],
            "tags": ["topic:ui"],
        });
        let cli = translate_args("add", &args).unwrap();
        assert_eq!(cli[0], "Likes dark mode");
        assert!(cli.windows(2).any(|w| w == ["--type", "preference"]));
        assert_eq!(cli.iter().filter(|s| *s == "--context").count(), 2);
        assert!(cli.iter().any(|s| s == "code/linggen"));
        assert!(cli.iter().any(|s| s == "ui"));
    }

    #[test]
    fn translate_search_requires_query() {
        assert!(translate_args("search", &json!({})).is_err());
        let cli = translate_args("search", &json!({"query": "docks", "limit": 5})).unwrap();
        assert_eq!(cli[0], "docks");
        assert!(cli.windows(2).any(|w| w == ["--limit", "5"]));
    }

    #[test]
    fn translate_delete_auto_confirms() {
        let cli = translate_args("delete", &json!({"id": "abc"})).unwrap();
        assert_eq!(cli, vec!["abc".to_string(), "--yes".to_string()]);
    }

    #[test]
    fn translate_forget_auto_confirms_and_takes_filters() {
        let cli = translate_args(
            "forget",
            &json!({"contexts": ["trip-japan-2026"], "older_than": "2026-01-01"}),
        )
        .unwrap();
        assert!(cli.windows(2).any(|w| w == ["--context", "trip-japan-2026"]));
        assert!(cli.windows(2).any(|w| w == ["--older-than", "2026-01-01"]));
        assert!(cli.iter().any(|s| s == "--yes"));
    }

    #[test]
    fn parse_stdout_handles_empty_single_and_ndjson() {
        assert_eq!(parse_stdout(b"").unwrap(), Value::Null);
        assert_eq!(parse_stdout(b"   \n\n ").unwrap(), Value::Null);
        assert_eq!(
            parse_stdout(b"{\"id\":\"1\"}").unwrap(),
            json!({"id": "1"})
        );
        assert_eq!(
            parse_stdout(b"{\"id\":\"1\"}\n{\"id\":\"2\"}\n").unwrap(),
            json!([{"id": "1"}, {"id": "2"}])
        );
    }

    #[test]
    fn parse_stdout_wraps_invalid_json_in_error() {
        let err = parse_stdout(b"not json at all").unwrap_err().to_string();
        assert!(err.contains("parsing provider stdout as JSON"));
    }

    #[test]
    fn resolve_provider_binary_prefers_skill_dir() {
        let tmp = std::env::temp_dir().join("linggen_memory_resolve_test");
        let bin_dir = tmp.join("bin");
        let _ = std::fs::create_dir_all(&bin_dir);
        let binary = bin_dir.join("ling-mem");
        std::fs::write(&binary, "").unwrap();

        let skill = bare_skill("mem", Some(vec!["memory".into()]), Some(tmp.clone()));
        let resolved = resolve_provider_binary(&skill).unwrap();
        assert_eq!(resolved, binary);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolve_provider_binary_falls_back_to_path() {
        // Skill with no on-disk binary → fall back to bare `ling-mem`.
        let skill = bare_skill("mem", Some(vec!["memory".into()]), None);
        let resolved = resolve_provider_binary(&skill).unwrap();
        assert_eq!(resolved, PathBuf::from("ling-mem"));
    }

    #[tokio::test]
    async fn dispatch_shells_out_and_parses_stdout() {
        // End-to-end: fake `ling-mem` shell script in a tmp skill dir.
        let tmp = std::env::temp_dir().join("linggen_memory_dispatch_e2e");
        let bin_dir = tmp.join("bin");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&bin_dir).unwrap();
        let script = bin_dir.join("ling-mem");
        // Echo a single-row JSON for any invocation.
        std::fs::write(
            &script,
            "#!/bin/sh\necho '{\"id\":\"row-1\",\"content\":\"hi\"}'\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script, perms).unwrap();
        }

        let skills = SkillManager::new();
        skills
            .insert_for_test(bare_skill(
                "linggen-memory",
                Some(vec!["memory".into()]),
                Some(tmp.clone()),
            ))
            .await;

        let result = dispatch(&skills, "search", json!({"query": "x"})).await.unwrap();
        assert_eq!(result, json!({"id": "row-1", "content": "hi"}));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn dispatch_surfaces_structured_stderr_error() {
        let tmp = std::env::temp_dir().join("linggen_memory_dispatch_err");
        let bin_dir = tmp.join("bin");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&bin_dir).unwrap();
        let script = bin_dir.join("ling-mem");
        std::fs::write(
            &script,
            "#!/bin/sh\necho '{\"error\":\"row not found\",\"code\":\"NOT_FOUND\"}' 1>&2\nexit 2\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script, perms).unwrap();
        }

        let skills = SkillManager::new();
        skills
            .insert_for_test(bare_skill(
                "linggen-memory",
                Some(vec!["memory".into()]),
                Some(tmp.clone()),
            ))
            .await;

        let err = dispatch(&skills, "get", json!({"id": "missing"}))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("NOT_FOUND"), "got: {err}");
        assert!(err.contains("row not found"), "got: {err}");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}

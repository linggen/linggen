//! HTTP dispatch for capability tools.
//!
//! A **capability tool** (e.g. `Memory_search`) has its schema defined by
//! the engine in `capabilities::CAPABILITIES` and its implementation
//! provided by a skill that `provides: [memory]` and declares an
//! `implements:` block in SKILL.md. This module does the routing: given
//! the tool name + JSON args, find the active provider's binding, combine
//! `base_url + tool_path`, POST the args, and parse the standard
//! `{ok, data} | {ok:false, error, code}` envelope.
//!
//! Autostart: on a connection refuse or timeout on the first try, spawn
//! the skill's declared `autostart` command and retry once. The daemon
//! outlives the Linggen process — we never auto-stop it.
//!
//! See:
//! - `../../linggen/doc/memory-spec.md` § HTTP dispatch contract
//! - `../../linggen/doc/skill-spec.md` § Skill daemons + capability
//!   implementation

use crate::engine::capabilities;
use crate::skills::{CapabilityImpl, SkillManager};
use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Per-call HTTP timeout. Matches `doc/memory-spec.md` § HTTP dispatch.
const DISPATCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Outer budget for `<binary> start`. The spawned command has its own
/// 10s internal budget; this ceiling avoids cancelling a start that's
/// about to succeed.
const AUTOSTART_TIMEOUT: Duration = Duration::from_secs(15);

/// Default fallback binary used when a skill's `implements.autostart` is
/// omitted. Matches the ships-with-the-memory-skill binary name.
const DEFAULT_AUTOSTART: &str = "ling-mem start";

/// Dispatch a capability-tool call to the active provider's daemon.
///
/// The tool's schema + tier already live in `engine::capabilities`.
/// This function's only job is to resolve `tool_name → url` via the
/// active skill's `implements:` block, POST, and return the unwrapped
/// response payload.
pub(crate) async fn dispatch(
    skills: &SkillManager,
    tool_name: &str,
    args: Value,
) -> Result<Value> {
    let (cap_name, _contract) = capabilities::capability_for_tool(tool_name)
        .ok_or_else(|| anyhow!("{tool_name} is not a capability tool"))?;

    let provider = skills.active_provider(cap_name).await.ok_or_else(|| {
        anyhow!(
            "No provider installed for capability `{cap_name}`. Install a skill \
             that declares `provides: [{cap_name}]` (e.g. the memory skill) \
             from the marketplace, then retry."
        )
    })?;

    let impl_block = provider
        .implements
        .as_ref()
        .and_then(|m| m.get(cap_name))
        .ok_or_else(|| {
            anyhow!(
                "Skill `{}` claims `provides: [{cap_name}]` but declares no \
                 `implements: {cap_name}:` block — can't dispatch {tool_name}.",
                provider.name
            )
        })?;

    let path = impl_block.tools.get(tool_name).ok_or_else(|| {
        anyhow!(
            "Skill `{}` does not expose `{tool_name}` in its `implements.{cap_name}.tools` \
             map — add it or use a different provider.",
            provider.name
        )
    })?;

    let url = join_url(&impl_block.base_url, path);

    match post_to_daemon(&url, &args).await {
        Ok(value) => Ok(value),
        Err(DispatchError::NoDaemon) => {
            autostart(provider.skill_dir.as_deref(), impl_block.autostart.as_deref())
                .await
                .with_context(|| {
                    format!(
                        "autostarting skill `{}` after first HTTP attempt to {url} failed",
                        provider.name
                    )
                })?;
            post_to_daemon(&url, &args).await.map_err(Into::into)
        }
        Err(DispatchError::Other(e)) => Err(e),
    }
}

/// Combine a skill's `base_url` with a per-tool path, tolerating a
/// trailing slash on the base or a missing leading slash on the path.
fn join_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    if path.starts_with('/') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
}

#[derive(Debug)]
enum DispatchError {
    /// The daemon isn't reachable — autostart + retry.
    NoDaemon,
    /// Any other error — surface to the model.
    Other(anyhow::Error),
}

impl From<DispatchError> for anyhow::Error {
    fn from(e: DispatchError) -> Self {
        match e {
            DispatchError::NoDaemon => anyhow!(
                "skill daemon is not reachable after autostart — check `ling-mem status` \
                 and the skill's install"
            ),
            DispatchError::Other(e) => e,
        }
    }
}

async fn post_to_daemon(url: &str, args: &Value) -> Result<Value, DispatchError> {
    let client = reqwest::Client::builder()
        .timeout(DISPATCH_TIMEOUT)
        .build()
        .map_err(|e| DispatchError::Other(anyhow!(e)))?;

    let response = match client.post(url).json(args).send().await {
        Ok(r) => r,
        Err(e) if e.is_connect() || e.is_timeout() => return Err(DispatchError::NoDaemon),
        Err(e) => {
            return Err(DispatchError::Other(
                anyhow::Error::from(e).context(format!("POST {url} failed")),
            ));
        }
    };

    let envelope: Value = response.json().await.map_err(|e| {
        DispatchError::Other(anyhow::Error::from(e).context("parsing daemon response as JSON"))
    })?;

    parse_envelope(envelope).map_err(DispatchError::Other)
}

/// Extract `data` from a success envelope or build a tool error from a
/// failure envelope. Non-conforming responses are treated as provider
/// bugs — providers must emit exactly one of the two documented shapes.
fn parse_envelope(envelope: Value) -> Result<Value> {
    let obj = envelope
        .as_object()
        .ok_or_else(|| anyhow!("daemon response is not a JSON object: {envelope}"))?;
    match obj.get("ok").and_then(|v| v.as_bool()) {
        Some(true) => Ok(obj.get("data").cloned().unwrap_or(Value::Null)),
        Some(false) => {
            let msg = obj
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            let code = obj.get("code").and_then(|v| v.as_str());
            match code {
                Some(c) => Err(anyhow!("skill provider error [{c}]: {msg}")),
                None => Err(anyhow!("skill provider error: {msg}")),
            }
        }
        None => Err(anyhow!("daemon response missing `ok` field: {envelope}")),
    }
}

/// Spawn `<binary> start` (or whatever the skill declared) and wait for
/// it to exit. The subprocess is expected to be idempotent — running it
/// when the daemon is already up should print "already running" and
/// exit 0.
async fn autostart(skill_dir: Option<&Path>, autostart_cmd: Option<&str>) -> Result<()> {
    let cmd_str = autostart_cmd.unwrap_or(DEFAULT_AUTOSTART);
    // Whitespace-split is deliberate: simple daemon-start commands like
    // `ling-mem start --port 9888` don't need quoting. Users who need
    // shell escaping should ship a wrapper script instead.
    let mut tokens = cmd_str.split_whitespace();
    let binary_name = tokens
        .next()
        .ok_or_else(|| anyhow!("empty autostart command"))?
        .to_string();
    let args: Vec<String> = tokens.map(String::from).collect();

    let binary = resolve_binary(skill_dir, &binary_name);
    let output = tokio::time::timeout(
        AUTOSTART_TIMEOUT,
        tokio::process::Command::new(&binary)
            .args(&args)
            .env("LINGGEN_DATA_DIR", crate::paths::linggen_home())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| {
        anyhow!(
            "`{} {}` did not complete within {}s",
            binary.display(),
            args.join(" "),
            AUTOSTART_TIMEOUT.as_secs()
        )
    })?
    .with_context(|| format!("spawning `{} {}`", binary.display(), args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let trimmed = stderr.trim();
        return Err(anyhow!(
            "`{} {}` exited with status {}{}",
            binary.display(),
            args.join(" "),
            output.status,
            if trimmed.is_empty() {
                String::new()
            } else {
                format!(": {trimmed}")
            }
        ));
    }
    Ok(())
}

/// Resolve an autostart binary: `$SKILL_DIR/bin/<name>` first (what the
/// skill's `install.sh` lays down), then fall back to the bare name on
/// `$PATH` so dev setups (`cargo install`) work without manual symlinks.
fn resolve_binary(skill_dir: Option<&Path>, binary_name: &str) -> PathBuf {
    if let Some(dir) = skill_dir {
        let candidate = dir.join("bin").join(binary_name);
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from(binary_name)
}

/// Expose a read-only view of a skill's capability binding so other
/// engine modules (permission prompts, UIs) can render stable info.
#[allow(dead_code)]
pub(crate) fn resolve_binding<'a>(
    skill: &'a crate::skills::Skill,
    capability: &str,
) -> Option<&'a CapabilityImpl> {
    skill.implements.as_ref().and_then(|m| m.get(capability))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn join_url_handles_trailing_slashes() {
        assert_eq!(
            join_url("http://localhost:9888", "/api/memory/search"),
            "http://localhost:9888/api/memory/search"
        );
        assert_eq!(
            join_url("http://localhost:9888/", "/api/memory/search"),
            "http://localhost:9888/api/memory/search"
        );
        assert_eq!(
            join_url("http://localhost:9888", "api/memory/search"),
            "http://localhost:9888/api/memory/search"
        );
    }

    #[test]
    fn parse_envelope_success_extracts_data() {
        let env = json!({"ok": true, "data": {"id": "abc"}});
        assert_eq!(parse_envelope(env).unwrap(), json!({"id": "abc"}));
    }

    #[test]
    fn parse_envelope_error_surfaces_code_and_message() {
        let env = json!({"ok": false, "error": "row not found", "code": "NOT_FOUND"});
        let err = parse_envelope(env).unwrap_err().to_string();
        assert!(err.contains("NOT_FOUND"), "got: {err}");
        assert!(err.contains("row not found"), "got: {err}");
    }

    #[test]
    fn parse_envelope_rejects_malformed() {
        assert!(parse_envelope(json!("string")).is_err());
        assert!(parse_envelope(json!({"data": 1})).is_err());
    }

    #[test]
    fn resolve_binary_prefers_skill_dir() {
        let tmp = std::env::temp_dir().join("linggen_cap_resolve_test_v2");
        let bin_dir = tmp.join("bin");
        let _ = std::fs::create_dir_all(&bin_dir);
        let binary = bin_dir.join("ling-mem");
        std::fs::write(&binary, "").unwrap();
        assert_eq!(resolve_binary(Some(&tmp), "ling-mem"), binary);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolve_binary_falls_back_to_path() {
        assert_eq!(
            resolve_binary(None, "ling-mem"),
            PathBuf::from("ling-mem")
        );
    }
}

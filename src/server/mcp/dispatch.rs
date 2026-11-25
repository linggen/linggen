//! What runs a branch: the dispatcher its catalogue entry names, with the
//! entry's timeout. Nothing here looks at a tool's name.

use super::catalog::{AgentOp, Dispatch, Render};
use super::{compose_dream_status, render_data, tool_content, McpDeps};
use ling_mem::catalog::Branch;
use serde_json::{json, Value};
use std::time::Duration;

/// Run `branch` with `args`; every outcome is a tool result (a failure is
/// one with `isError`).
pub(super) async fn run(deps: &McpDeps<'_>, branch: &Branch<Dispatch>, args: Value) -> Value {
    match branch.dispatch {
        Dispatch::Bridge { module, op, render } => {
            bridge(deps, module, op, render, args, branch.timeout_ms).await
        }
        Dispatch::Agent(op) => agent(deps, op, args).await,
        Dispatch::DreamStatus => dream_status(deps).await,
        Dispatch::DreamRun => dream_run(deps, args).await,
        Dispatch::Memory(verb) => memory(verb, args, branch.timeout_ms).await,
        Dispatch::App(op) => super::app::run(deps, op, args).await,
    }
}

async fn bridge(
    deps: &McpDeps<'_>,
    module: &str,
    op: &str,
    render: Render,
    args: Value,
    timeout_ms: u64,
) -> Value {
    let res = deps.bridge.call_value(module, op, args, timeout_ms).await;
    if res.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        let data = res.get("data").cloned().unwrap_or(Value::Null);
        return render_data(render, &data);
    }
    let code = res
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("upstream_error");
    let message = res.get("message").and_then(Value::as_str).unwrap_or("");
    if let Some(why) = crate::cannot_run::CannotRun::of_bridge(code) {
        return cannot_run(&why, NOT_CONNECTED);
    }
    tool_content(bridge_failure(code, message), true)
}

/// What a calling agent is told when the extension is not connected.
const NOT_CONNECTED: &str = "browser not connected — the linggen-browser extension must be \
    running and connected in Chrome. Ask the user to install or enable it, then retry.";

/// A call that cannot run, for a reason a caller acts on: the words, and
/// the reason as a field (`structuredContent.cannot_run`).
fn cannot_run(why: &crate::cannot_run::CannotRun, text: &str) -> Value {
    let mut res = tool_content(text.to_string(), true);
    res["structuredContent"] = json!({ "cannot_run": why.fact() });
    res
}

/// Any other bridge refusal, in words a calling agent can act on.
fn bridge_failure(code: &str, message: &str) -> String {
    match code {
        "not_permitted" => format!(
            "not_permitted: {message} — Chrome did not grant permission for this site \
            (the user declined it in Chrome's prompt); ask the user before retrying."
        ),
        "element_gone" => format!(
            "element_gone: {message} — the page changed since it was read; read it \
            again (browser_read op=page) and use the new refs."
        ),
        _ => format!("{code}: {message}"),
    }
}

async fn agent(deps: &McpDeps<'_>, op: AgentOp, args: Value) -> Value {
    let Some(state) = deps.state else {
        return tool_content(
            "agent_run is unavailable in this context (no daemon state)".to_string(),
            true,
        );
    };
    let field = |key: &str| args.get(key).and_then(Value::as_str);
    let session = field("session_id").unwrap_or_default();
    let result = match op {
        AgentOp::Status => crate::server::mcp_agent::status(state, session).await,
        AgentOp::Stop => crate::server::mcp_agent::stop(state, session).await,
        AgentOp::Permit => {
            crate::server::mcp_agent::permit(
                state,
                session,
                field("request_id").unwrap_or_default(),
                field("decision").unwrap_or_default(),
                field("scope"),
            )
            .await
        }
        AgentOp::Readiness => Ok(crate::server::mcp_agent::readiness(state).await),
        AgentOp::Close => {
            let remove = args
                .get("remove_worktree")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            crate::server::mcp_agent::close(state, session, remove).await
        }
        AgentOp::Run => {
            crate::server::mcp_agent::run(
                state,
                field("agent"),
                field("prompt").unwrap_or_default(),
                field("write_dir"),
                field("session_id"),
                field("model"),
                field("permission_mode"),
                field("host"),
                args.get("model_fallback").and_then(Value::as_bool),
                field("work"),
                args.get("grants"),
                deps.read_origin,
            )
            .await
        }
    };
    match result {
        Ok(text) => tool_content(text, false),
        Err(msg) => tool_content(msg, true),
    }
}

async fn dream_status(deps: &McpDeps<'_>) -> Value {
    let Some(state) = deps.state else {
        return tool_content(
            "memory_upkeep op=dream_status is unavailable in this context (no daemon state)"
                .to_string(),
            true,
        );
    };
    match compose_dream_status(state).await {
        Ok(status) => tool_content(status.to_string(), false),
        Err(e) => tool_content(
            format!("dream status unavailable — memory unreachable: {e:#}"),
            true,
        ),
    }
}

async fn dream_run(deps: &McpDeps<'_>, args: Value) -> Value {
    let Some(state) = deps.state else {
        return tool_content(
            "memory_upkeep op=dream_run is unavailable in this context (no daemon state)"
                .to_string(),
            true,
        );
    };
    let day = args.get("day").and_then(Value::as_str).map(str::to_string);
    use crate::server::api::missions::{trigger_mission_core, TriggerOutcome};
    match trigger_mission_core(state, "dream", None, day.clone(), false).await {
        TriggerOutcome::Started { session_id } => tool_content(
            json!({
                "started": true,
                "session_id": session_id,
                "day": day,
                "note": "dream runs take minutes — read memory_upkeep op=dream_status later; if it reports a failed run, show last_run_error to the user",
            })
            .to_string(),
            false,
        ),
        TriggerOutcome::InFlight => tool_content(
            json!({
                "started": false,
                "in_flight": true,
                "note": "a dream run is already in flight — read memory_upkeep op=dream_status later",
            })
            .to_string(),
            false,
        ),
        TriggerOutcome::NotFound => tool_content(
            "dream mission not found — the engine's built-in dream mission is missing \
             (check ~/.linggen/missions/dream or reinstall Linggen)"
                .to_string(),
            true,
        ),
        TriggerOutcome::BadDay(d) => {
            tool_content(format!("invalid day '{d}' — expected YYYY-MM-DD"), true)
        }
        TriggerOutcome::Internal(e) => tool_content(format!("dream trigger failed: {e}"), true),
    }
}

/// A memory branch, through the memory this process hosts: the same
/// adapters memory's own server runs a branch with
/// (`ling_mem::catalog::memory`), as the owner.
pub(super) async fn memory(verb: &'static str, mut args: Value, timeout_ms: u64) -> Value {
    use ling_mem::catalog::memory::{finish, prepare, read_envelope};
    prepare(&mut args);
    let wait = Duration::from_millis(timeout_ms);
    use crate::cannot_run::CannotRun;
    let answer = crate::memory_host::post_verb(ling_mem::Who::Owner, verb, args, wait).await;
    let envelope = match answer {
        Ok((status, envelope)) => match CannotRun::of_memory_answer(status, &envelope) {
            Some(why) => return unavailable(&why),
            None => envelope,
        },
        // No answer in time is what memory's own runner says it is: the
        // verb may still finish, so a write may have landed.
        Err(e) if e.is::<crate::memory_host::NoAnswerInTime>() => {
            let line = ling_mem::catalog::memory::timed_out_line(verb, wait);
            return tool_content(line, true);
        }
        Err(e) => match CannotRun::of_memory_error(&e.to_string()) {
            Some(why) => return unavailable(&why),
            None => return tool_content(e.to_string(), true),
        },
    };
    // Like other hosted tool refusals, this is isError, not an RPC failure.
    // Preserve the elapsed-time fact without treating a warming store as down.
    match read_envelope(envelope) {
        Ok(mut data) => {
            finish(verb, &mut data);
            tool_content(data.to_string(), false)
        }
        Err(message) => tool_content(message, true),
    }
}

/// A store that is not there: what it said, then what to do.
fn unavailable(why: &crate::cannot_run::CannotRun) -> Value {
    let detail = match why {
        crate::cannot_run::CannotRun::StoreUnavailable { detail } => detail.as_str(),
        _ => "",
    };
    tool_content(format!("{detail} — {}", why.line()), true)
}

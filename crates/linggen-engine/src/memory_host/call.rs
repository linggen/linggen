//! A memory verb, called in-process: how the rest of the engine reaches the
//! store it hosts (the door, the built-in memory helpers, the core block).
//!
//! The verb is answered as its route `/api/memory/<verb>` answers it
//! (`ling_mem::http::verb::answer`: the caller's [`Who`] applied, the body
//! checked as the route checks it, then `Memory::call`), so every status,
//! envelope and refusal is the one the loopback request used to bring back.
//!
//! It runs on memory's own runtime: the engine's task only awaits the join.
//! That is safe because the store's futures never run on an engine thread
//! (memory's runtime has workers of its own and cannot be starved by the
//! engine, nor stall it), and because a caller that stops waiting drops only
//! the join handle — the verb finishes on memory's runtime, as a request
//! did that `routes` had handed there.

use super::HOST;
use anyhow::{anyhow, Result};
use ling_mem::Who;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::task::JoinHandle;

/// What a verb answered: memory's status and its envelope.
pub type Answer = (u16, Value);

/// Run `verb` with `args` for `who`, waiting up to `timeout`. `Ok` whenever
/// memory answered — a refusal is memory's envelope under its own status,
/// and memory that is not open answers 503 `MEMORY_UNAVAILABLE` with the
/// reason, as its routes do. `Err` when there was no answer: no memory in
/// this process, or none in time.
pub async fn post_verb(who: Who, verb: &str, args: Value, timeout: Duration) -> Result<Answer> {
    let task = start(who, verb, args)?;
    match tokio::time::timeout(timeout, task).await {
        Err(_) => Err(no_answer()),
        Ok(Err(e)) => Err(anyhow!("memory request failed: {e}")),
        Ok(Ok(answer)) => answer,
    }
}

/// [`post_verb`] for a caller that is not async (prompt assembly): `None`
/// when no answer came in time. Waiting on an engine worker hands that
/// worker's queued tasks to another thread first.
pub fn post_verb_blocking(
    who: Who,
    verb: &str,
    args: Value,
    timeout: Duration,
) -> Option<Result<Answer>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let task = match start(who, verb, args) {
        Ok(task) => task,
        Err(e) => return Some(Err(e)),
    };
    let handle = tokio::runtime::Handle::try_current().ok();
    let on = target_runtime(handle.as_ref());
    on.spawn(async move {
        let answer = task
            .await
            .unwrap_or_else(|e| Err(anyhow!("memory request failed: {e}")));
        let _ = tx.send(answer);
    });
    let wait = || rx.recv_timeout(timeout).ok();
    match handle {
        Some(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(wait)
        }
        _ => wait(),
    }
}

/// A memory that did not answer in time — typed, so a caller tells it from
/// one that is not there: the verb may still finish on memory's runtime.
#[derive(Debug)]
pub struct NoAnswerInTime;

impl std::fmt::Display for NoAnswerInTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("memory is not reachable — check `ling status`")
    }
}

impl std::error::Error for NoAnswerInTime {}

fn no_answer() -> anyhow::Error {
    anyhow::Error::new(NoAnswerInTime)
}

/// The verb, started on memory's runtime.
fn start(who: Who, verb: &str, args: Value) -> Result<JoinHandle<Result<Answer>>> {
    let verb = verb.to_string();
    let run = |state: ling_mem::http::state::SharedState| async move {
        let response = ling_mem::http::verb::answer(state, who, &verb, args).await;
        envelope_of(response).await
    };
    match target()? {
        Target::Store(state, runtime) => Ok(runtime.spawn(run(state))),
        Target::NotOpen(why, runtime) => Ok(runtime.spawn(async move { Ok(unavailable(&why)) })),
        #[cfg(any(test, feature = "test-support"))]
        Target::Silent(runtime) => Ok(runtime.spawn(std::future::pending::<Result<Answer>>())),
    }
}

/// Where a verb goes.
enum Target {
    Store(ling_mem::http::state::SharedState, tokio::runtime::Handle),
    NotOpen(String, tokio::runtime::Handle),
    /// Tests only: a store that never answers.
    #[cfg(any(test, feature = "test-support"))]
    Silent(tokio::runtime::Handle),
}

fn target() -> Result<Target> {
    #[cfg(any(test, feature = "test-support"))]
    if let Ok(t) = TEST_HOST.try_with(TestHost::target) {
        return t;
    }
    let host = HOST
        .get()
        .ok_or_else(|| anyhow!("memory is not hosted by this process"))?;
    let runtime = host.runtime.clone();
    Ok(match &host.served {
        Ok(served) => Target::Store(served.state.clone(), runtime),
        Err(why) => Target::NotOpen(why.clone(), runtime),
    })
}

/// The runtime a blocking wait's relay task runs on: memory's when this
/// process hosts it, else (a test's store) the caller's.
fn target_runtime(current: Option<&tokio::runtime::Handle>) -> tokio::runtime::Handle {
    match (HOST.get(), current) {
        (Some(host), _) => host.runtime.clone(),
        (None, Some(h)) => h.clone(),
        (None, None) => super::runtime().handle().clone(),
    }
}

/// Memory's answer as its envelope. A body that is not the envelope (the
/// router's empty 404, axum's plain-text 422) is given the envelope shape,
/// its status as code; a 2xx that is not the envelope is memory
/// misbehaving, and an error. Memory's headers are not passed on.
async fn envelope_of(response: axum::response::Response) -> Result<Answer> {
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .map_err(|e| anyhow!("reading memory's answer: {e}"))?;
    let body = String::from_utf8_lossy(&bytes);
    if let Ok(envelope @ Value::Object(_)) = serde_json::from_str::<Value>(&body) {
        return Ok((status, envelope));
    }
    let trimmed = body.trim();
    let shown = if trimmed.is_empty() {
        "<empty body>"
    } else {
        trimmed
    };
    let message = format!("ling-mem error [{status}]: {shown}");
    if (200..300).contains(&status) {
        return Err(anyhow!(message));
    }
    Ok((
        status,
        json!({ "ok": false, "error": message, "code": format!("HTTP_{status}") }),
    ))
}

/// What memory's routes answer while the store is not open.
fn unavailable(why: &str) -> Answer {
    (
        503,
        json!({ "ok": false, "error": why, "code": "MEMORY_UNAVAILABLE" }),
    )
}

/// Tests only: what [`post_verb`] reaches inside `TEST_HOST.scope(host, fut)`,
/// for that future alone — a scratch store, a store that never answers, one
/// that failed to open, or none at all — without touching the process-wide
/// host.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone)]
pub enum TestHost {
    Store(ling_mem::http::state::SharedState),
    Silent,
    NotOpen(String),
    Absent,
}

#[cfg(any(test, feature = "test-support"))]
impl TestHost {
    fn target(&self) -> Result<Target> {
        let here = tokio::runtime::Handle::try_current()
            .unwrap_or_else(|_| super::runtime().handle().clone());
        match self {
            TestHost::Store(state) => Ok(Target::Store(state.clone(), here)),
            TestHost::Silent => Ok(Target::Silent(here)),
            TestHost::NotOpen(why) => Ok(Target::NotOpen(why.clone(), here)),
            TestHost::Absent => Err(anyhow!("memory is not hosted by this process")),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
tokio::task_local! {
    pub static TEST_HOST: TestHost;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    async fn blocking_at(host: TestHost, timeout: Duration) -> Option<Result<Answer>> {
        TEST_HOST
            .scope(host, async move {
                post_verb_blocking(Who::Owner, "count", json!({}), timeout)
            })
            .await
    }

    /// Prompt assembly is never held past its bound by a store that does not
    /// answer; one that is not open answers as its routes do; with none in
    /// this process the caller hears so at once.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_blocking_call_keeps_its_bound() {
        let bound = Duration::from_millis(300);
        let started = Instant::now();
        assert!(blocking_at(TestHost::Silent, bound).await.is_none());
        let waited = started.elapsed();
        assert!(waited >= bound && waited < bound * 4, "{waited:?}");

        let not_open = blocking_at(TestHost::NotOpen("locked".into()), bound).await;
        assert_eq!(
            not_open.unwrap().unwrap(),
            (
                503,
                json!({"ok": false, "error": "locked", "code": "MEMORY_UNAVAILABLE"})
            )
        );
        let absent = blocking_at(TestHost::Absent, bound).await.unwrap();
        assert_eq!(
            absent.unwrap_err().to_string(),
            "memory is not hosted by this process"
        );
    }

    #[tokio::test]
    async fn an_async_call_gives_up_after_its_timeout() {
        let call = post_verb(Who::Owner, "count", json!({}), Duration::from_millis(50));
        let err = TEST_HOST.scope(TestHost::Silent, call).await.unwrap_err();
        assert!(err.is::<NoAnswerInTime>(), "{err}");
        assert_eq!(
            err.to_string(),
            "memory is not reachable — check `ling status`"
        );
    }
}

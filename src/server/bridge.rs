//! Browser bridge — the daemon side of `doc/browser-bridge-spec.md`.
//!
//! The `linggen-browser` extension dials `GET /api/bridge/socket` and holds the
//! WebSocket open. Skills never speak WebSocket: they `POST /api/bridge/call`,
//! the daemon brokers one request over the socket and blocks until the
//! extension answers (or times out). `GET /api/bridge/status` reports whether an
//! extension is attached and which modules it offers.
//!
//! Cookies never cross this bridge — only parsed result objects. Reads are
//! on-demand; the daemon never pushes unsolicited work.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header::ORIGIN, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::server::ServerState;

const BRIDGE_VERSION: &str = "1";
const DEFAULT_TIMEOUT_MS: u64 = 20_000;

/// One module the connected extension offers (e.g. `x`).
#[derive(Clone, Serialize)]
struct ModuleState {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    ready: bool,
}

/// A `res` frame, delivered back to a waiting `call`.
struct ResData {
    ok: bool,
    data: Option<Value>,
    code: Option<String>,
    message: Option<String>,
}

impl ResData {
    fn err(code: &str, message: &str) -> Self {
        Self {
            ok: false,
            data: None,
            code: Some(code.into()),
            message: Some(message.into()),
        }
    }

    /// Shape returned to the skill: `{ok:true, data}` or `{ok:false, code, message}`.
    fn into_value(self) -> Value {
        if self.ok {
            json!({ "ok": true, "data": self.data.unwrap_or(Value::Null) })
        } else {
            json!({ "ok": false, "code": self.code, "message": self.message })
        }
    }
}

#[derive(Default)]
struct HubInner {
    /// Outbound frame sender for the live socket; `None` when disconnected.
    tx: Option<mpsc::UnboundedSender<String>>,
    /// Connection generation — guards `detach` so a superseded socket can't
    /// clear the connection that replaced it.
    generation: u64,
    ext_version: Option<String>,
    modules: Vec<ModuleState>,
}

/// Where a request's `progress` lines go — the caller's live status.
type ProgressFn = Box<dyn Fn(String) + Send + Sync>;

/// Shared bridge state: the single connected extension plus in-flight requests.
pub struct BridgeHub {
    inner: Mutex<HubInner>,
    pending: Mutex<HashMap<String, oneshot::Sender<ResData>>>,
    /// Requests whose caller wants `progress` lines. A plain mutex: the
    /// registration is dropped by a guard, which cannot await.
    progress: std::sync::Mutex<HashMap<String, ProgressFn>>,
    seq: AtomicU64,
}

/// Removes a request's progress listener however the call ends — answered,
/// timed out, or its future dropped by a cancelled tool.
struct ProgressGuard<'a> {
    hub: &'a BridgeHub,
    id: String,
}

/// One log line per brokered op, however it ends — answered, refused, timed
/// out, or abandoned when its caller's future is dropped (a tool the user
/// stopped). Without it the log cannot say whether a call ever reached the
/// extension or what came back.
struct CallTrace<'a> {
    module: &'a str,
    op: &'a str,
    id: String,
    started: Instant,
    done: bool,
}

impl<'a> CallTrace<'a> {
    fn new(module: &'a str, op: &'a str, id: &str) -> Self {
        Self {
            module,
            op,
            id: id.to_string(),
            started: Instant::now(),
            done: false,
        }
    }

    fn finish(mut self, res: &ResData) {
        self.done = true;
        let ms = self.started.elapsed().as_millis() as u64;
        let (module, op, id) = (self.module, self.op, self.id.as_str());
        if res.ok {
            tracing::info!(module, op, id, ms, outcome = "ok", "bridge/call");
            return;
        }
        let outcome = res.code.as_deref().unwrap_or("error");
        let detail = res.message.as_deref().unwrap_or("");
        if expected_refusal(outcome) {
            tracing::info!(module, op, id, ms, outcome, detail, "bridge/call");
        } else {
            tracing::warn!(module, op, id, ms, outcome, detail, "bridge/call");
        }
    }
}

impl Drop for CallTrace<'_> {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let ms = self.started.elapsed().as_millis() as u64;
        let (module, op, id) = (self.module, self.op, self.id.as_str());
        tracing::info!(module, op, id, ms, outcome = "abandoned", "bridge/call");
    }
}

/// Refusals that are the page or the person answering, not the bridge failing.
fn expected_refusal(code: &str) -> bool {
    matches!(code, "not_permitted" | "element_gone")
}

impl Drop for ProgressGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut map) = self.hub.progress.lock() {
            map.remove(&self.id);
        }
    }
}

impl BridgeHub {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HubInner::default()),
            pending: Mutex::new(HashMap::new()),
            progress: std::sync::Mutex::new(HashMap::new()),
            seq: AtomicU64::new(1),
        }
    }

    /// Register a new socket's sender. A second connection supersedes the first:
    /// dropping the prior sender closes its channel, ending its loop.
    async fn attach(&self, tx: mpsc::UnboundedSender<String>) -> u64 {
        let mut inner = self.inner.lock().await;
        inner.generation += 1;
        inner.tx = Some(tx);
        inner.generation
    }

    /// Clear connection state — but only if `generation` is still current, so a
    /// superseded socket's teardown doesn't wipe its replacement.
    async fn detach(&self, generation: u64) {
        let mut inner = self.inner.lock().await;
        if inner.generation == generation {
            inner.tx = None;
            inner.ext_version = None;
            inner.modules.clear();
            tracing::info!(generation, "bridge/disconnected");
        }
    }

    async fn send_frame(&self, frame: Value) -> bool {
        let inner = self.inner.lock().await;
        match &inner.tx {
            Some(tx) => tx.send(frame.to_string()).is_ok(),
            None => false,
        }
    }

    fn next_id(&self) -> String {
        format!("req-{}", self.seq.fetch_add(1, Ordering::Relaxed))
    }

    /// Dispatch one inbound frame from the extension.
    async fn on_frame(&self, text: &str) {
        let Ok(v) = serde_json::from_str::<Value>(text) else {
            return;
        };
        match v.get("t").and_then(Value::as_str) {
            Some("hello") => self.on_hello(&v).await,
            Some("res") => self.on_res(&v).await,
            Some("progress") => self.on_progress(&v),
            Some("status") => self.merge_modules(&v).await,
            _ => {}
        }
    }

    async fn on_hello(&self, v: &Value) {
        {
            let mut inner = self.inner.lock().await;
            inner.ext_version = v
                .get("ext_version")
                .and_then(Value::as_str)
                .map(String::from);
            inner.modules = parse_modules(v);
            let modules = inner
                .modules
                .iter()
                .map(|m| format!("{}@{}", m.id, m.version.as_deref().unwrap_or("?")))
                .collect::<Vec<_>>()
                .join(",");
            let ext_version = inner.ext_version.as_deref().unwrap_or("?");
            tracing::info!(ext_version, modules = %modules, "bridge/connected");
        }
        self.send_frame(json!({ "t": "ready", "bridge_version": BRIDGE_VERSION }))
            .await;
    }

    async fn on_res(&self, v: &Value) {
        let Some(id) = v.get("id").and_then(Value::as_str) else {
            return;
        };
        let waiter = self.pending.lock().await.remove(id);
        let Some(waiter) = waiter else {
            tracing::debug!(id, "bridge/res for a call no longer waiting — dropped");
            return;
        };
        let _ = waiter.send(ResData {
            ok: v.get("ok").and_then(Value::as_bool).unwrap_or(false),
            data: v.get("data").cloned(),
            code: v.get("code").and_then(Value::as_str).map(String::from),
            message: v.get("message").and_then(Value::as_str).map(String::from),
        });
    }

    /// A `progress` frame: the op is waiting on something — the user's OK in
    /// the approval popup, or another browser action ahead of it.
    fn on_progress(&self, v: &Value) {
        let (Some(id), Some(text)) = (
            v.get("id").and_then(Value::as_str),
            v.get("text").and_then(Value::as_str),
        ) else {
            return;
        };
        let stage = v.get("stage").and_then(Value::as_str).unwrap_or("");
        tracing::info!(id, stage, text, "bridge/progress");
        if let Ok(map) = self.progress.lock() {
            if let Some(report) = map.get(id) {
                report(text.to_string());
            }
        }
    }

    async fn merge_modules(&self, v: &Value) {
        let updates = parse_modules(v);
        let mut inner = self.inner.lock().await;
        for m in updates {
            match inner.modules.iter_mut().find(|e| e.id == m.id) {
                Some(existing) => existing.ready = m.ready,
                None => inner.modules.push(m),
            }
        }
    }

    /// Broker one read: enqueue a `req`, wait for the matching `res` or timeout.
    async fn call(
        &self,
        module: &str,
        op: &str,
        params: Value,
        timeout_ms: u64,
        on_progress: Option<ProgressFn>,
    ) -> ResData {
        let id = self.next_id();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);
        let _guard = on_progress.map(|report| {
            if let Ok(mut map) = self.progress.lock() {
                map.insert(id.clone(), report);
            }
            ProgressGuard {
                hub: self,
                id: id.clone(),
            }
        });

        let trace = CallTrace::new(module, op, &id);
        let frame = json!({ "t": "req", "id": id, "module": module, "op": op, "params": params });
        let res = self.exchange(&id, frame, rx, timeout_ms).await;
        trace.finish(&res);
        res
    }

    /// Send one `req` and wait for its `res`, the timeout, or a dropped link.
    async fn exchange(
        &self,
        id: &str,
        frame: Value,
        rx: oneshot::Receiver<ResData>,
        timeout_ms: u64,
    ) -> ResData {
        if !self.send_frame(frame).await {
            self.pending.lock().await.remove(id);
            return ResData::err("no_bridge", "no browser extension is connected");
        }
        match tokio::time::timeout(Duration::from_millis(timeout_ms), rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => ResData::err("no_bridge", "bridge connection dropped"),
            Err(_) => {
                self.pending.lock().await.remove(id);
                ResData::err("timeout", "extension did not respond in time")
            }
        }
    }

    /// Broker one op for an in-process caller (the engine's `Browser_*`
    /// tools). Same envelope the HTTP `call` surface returns:
    /// `{ok:true, data}` or `{ok:false, code, message}`.
    pub async fn call_value(
        &self,
        module: &str,
        op: &str,
        params: Value,
        timeout_ms: u64,
    ) -> Value {
        self.call(module, op, params, timeout_ms, None)
            .await
            .into_value()
    }

    /// [call_value], with the op's `progress` lines handed to [on_progress]
    /// as they arrive — so a caller can show "waiting for your OK" instead of
    /// a bare spinner.
    pub async fn call_value_reporting(
        &self,
        module: &str,
        op: &str,
        params: Value,
        timeout_ms: u64,
        on_progress: impl Fn(String) + Send + Sync + 'static,
    ) -> Value {
        self.call(module, op, params, timeout_ms, Some(Box::new(on_progress)))
            .await
            .into_value()
    }

    async fn status(&self) -> Value {
        let inner = self.inner.lock().await;
        json!({
            "connected": inner.tx.is_some(),
            "ext_version": inner.ext_version,
            "modules": inner.modules,
        })
    }
}

impl Default for BridgeHub {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_modules(v: &Value) -> Vec<ModuleState> {
    let Some(arr) = v.get("modules").and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|m| {
            let id = m.get("id").and_then(Value::as_str)?.to_string();
            Some(ModuleState {
                id,
                version: m.get("version").and_then(Value::as_str).map(String::from),
                ready: m.get("ready").and_then(Value::as_bool).unwrap_or(true),
            })
        })
        .collect()
}

/// Only a browser extension (or a non-browser local tool with no Origin) may
/// attach. A web page's http(s) Origin is rejected so a random site can't reach
/// the loopback socket. TODO: pin the published extension id once it exists.
fn origin_allowed(headers: &HeaderMap) -> bool {
    match headers.get(ORIGIN) {
        None => true,
        Some(value) => value
            .to_str()
            .map(|o| o.starts_with("chrome-extension://"))
            .unwrap_or(false),
    }
}

async fn run_socket(socket: WebSocket, hub: Arc<BridgeHub>) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let generation = hub.attach(tx).await;

    loop {
        tokio::select! {
            outbound = rx.recv() => match outbound {
                Some(text) => {
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                None => break, // superseded by a newer connection
            },
            inbound = stream.next() => match inbound {
                Some(Ok(Message::Text(t))) => hub.on_frame(t.as_str()).await,
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {} // ignore binary/ping/pong (axum auto-pongs)
                Some(Err(_)) => break,
            },
        }
    }

    hub.detach(generation).await;
}

/// `GET /api/bridge/socket` — the extension's WebSocket endpoint.
pub(crate) async fn socket_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
) -> Response {
    if !origin_allowed(&headers) {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    let hub = state.bridge.clone();
    ws.on_upgrade(move |socket| run_socket(socket, hub))
}

#[derive(Deserialize)]
pub(crate) struct CallRequest {
    module: String,
    op: String,
    #[serde(default)]
    params: Value,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

/// `POST /api/bridge/call` — skills broker one read through the bridge.
pub(crate) async fn call_handler(
    State(state): State<Arc<ServerState>>,
    Json(req): Json<CallRequest>,
) -> impl IntoResponse {
    // Ceiling fits the slowest legitimate op: the x `targets` roster pull is
    // batched + retried in the extension (two paced searches, one 12s-backoff
    // retry each) and can legitimately run ~2 minutes. A 60s cap silently
    // starved it — the daemon gave up while the extension was still working.
    let timeout_ms = req
        .timeout_ms
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .clamp(1_000, 180_000);
    let res = state
        .bridge
        .call(&req.module, &req.op, req.params, timeout_ms, None)
        .await;
    Json(res.into_value())
}

/// `GET /api/bridge/status` — is the bridge connected, and which modules?
pub(crate) async fn status_handler(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    Json(state.bridge.status().await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::LockExt;

    #[tokio::test]
    async fn progress_reaches_its_caller_and_leaves_with_the_call() {
        let hub = BridgeHub::new();
        let (tx, _rx) = mpsc::unbounded_channel::<String>();
        hub.attach(tx).await;
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        let call = hub.call(
            "control",
            "tabs",
            json!({}),
            1_000,
            Some(Box::new(move |line| sink.lock_ok().push(line))),
        );
        let extension = async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            hub.on_frame(
                r#"{"t":"progress","id":"req-1","stage":"approval","text":"Waiting for your OK"}"#,
            )
            .await;
            hub.on_frame(r#"{"t":"progress","id":"req-9","text":"another call's line"}"#)
                .await;
            hub.on_frame(r#"{"t":"res","id":"req-1","ok":true,"data":{}}"#)
                .await;
        };
        let (res, _) = tokio::join!(call, extension);
        assert!(res.ok);
        assert_eq!(*seen.lock_ok(), vec!["Waiting for your OK".to_string()]);
        assert!(
            hub.progress.lock_ok().is_empty(),
            "the listener leaves with the call"
        );
    }
}

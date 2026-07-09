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
use std::time::Duration;

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
        Self { ok: false, data: None, code: Some(code.into()), message: Some(message.into()) }
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

/// Shared bridge state: the single connected extension plus in-flight requests.
pub struct BridgeHub {
    inner: Mutex<HubInner>,
    pending: Mutex<HashMap<String, oneshot::Sender<ResData>>>,
    seq: AtomicU64,
}

impl BridgeHub {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HubInner::default()),
            pending: Mutex::new(HashMap::new()),
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
        let Ok(v) = serde_json::from_str::<Value>(text) else { return };
        match v.get("t").and_then(Value::as_str) {
            Some("hello") => self.on_hello(&v).await,
            Some("res") => self.on_res(&v).await,
            Some("status") => self.merge_modules(&v).await,
            _ => {}
        }
    }

    async fn on_hello(&self, v: &Value) {
        {
            let mut inner = self.inner.lock().await;
            inner.ext_version = v.get("ext_version").and_then(Value::as_str).map(String::from);
            inner.modules = parse_modules(v);
        }
        self.send_frame(json!({ "t": "ready", "bridge_version": BRIDGE_VERSION })).await;
    }

    async fn on_res(&self, v: &Value) {
        let Some(id) = v.get("id").and_then(Value::as_str) else { return };
        let waiter = self.pending.lock().await.remove(id);
        let Some(waiter) = waiter else { return };
        let _ = waiter.send(ResData {
            ok: v.get("ok").and_then(Value::as_bool).unwrap_or(false),
            data: v.get("data").cloned(),
            code: v.get("code").and_then(Value::as_str).map(String::from),
            message: v.get("message").and_then(Value::as_str).map(String::from),
        });
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
    async fn call(&self, module: &str, op: &str, params: Value, timeout_ms: u64) -> ResData {
        let id = self.next_id();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);

        let frame = json!({ "t": "req", "id": id, "module": module, "op": op, "params": params });
        if !self.send_frame(frame).await {
            self.pending.lock().await.remove(&id);
            return ResData::err("no_bridge", "no browser extension is connected");
        }

        match tokio::time::timeout(Duration::from_millis(timeout_ms), rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => ResData::err("no_bridge", "bridge connection dropped"),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                ResData::err("timeout", "extension did not respond in time")
            }
        }
    }

    /// Broker one op for an in-process caller (the engine's `Browser_*`
    /// tools). Same envelope the HTTP `call` surface returns:
    /// `{ok:true, data}` or `{ok:false, code, message}`.
    pub async fn call_value(&self, module: &str, op: &str, params: Value, timeout_ms: u64) -> Value {
        self.call(module, op, params, timeout_ms).await.into_value()
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
    let Some(arr) = v.get("modules").and_then(Value::as_array) else { return Vec::new() };
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
    let timeout_ms = req.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).clamp(1_000, 60_000);
    let res = state.bridge.call(&req.module, &req.op, req.params, timeout_ms).await;
    Json(res.into_value())
}

/// `GET /api/bridge/status` — is the bridge connected, and which modules?
pub(crate) async fn status_handler(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    Json(state.bridge.status().await)
}

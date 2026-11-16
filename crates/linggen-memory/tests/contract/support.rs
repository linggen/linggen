//! One hosted memory per test: a scratch store in a temp dir, the real
//! router (`build_hosted_router`) served on a free loopback port, the
//! deterministic embedder. Requests go over real HTTP, as the engine's own
//! callers send them today (the MCP endpoint posts back to itself).

use ling_mem::http::state::SharedState;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::Path;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub struct Mem {
    /// Where the routes answer, e.g. `http://127.0.0.1:50123` (plus the
    /// prefix the store was started under).
    pub base: String,
    pub state: SharedState,
    pub dir: tempfile::TempDir,
    prefix: &'static str,
    addr: SocketAddr,
    server: Option<JoinHandle<()>>,
}

impl Mem {
    /// A store whose routes answer at the root of its port.
    pub async fn start() -> Self {
        Self::start_under("").await
    }

    /// A store whose routes answer under `prefix` (`/mem` — where a replica
    /// expects its primary's).
    pub async fn start_under(prefix: &'static str) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        Self::open(dir, prefix).await
    }

    /// A store over `dir` as it already is — e.g. with a `.replica.json`
    /// written before it opens.
    pub async fn open(dir: tempfile::TempDir, prefix: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}{prefix}");
        let state = ling_mem::hosted::open_quiet_fake(dir.path(), base.clone())
            .await
            .expect("open the scratch store");
        let mut me = Self {
            base,
            state,
            dir,
            prefix,
            addr,
            server: None,
        };
        me.serve(listener);
        me
    }

    fn serve(&mut self, listener: TcpListener) {
        let mut app = ling_mem::http::build_hosted_router(self.state.clone());
        if !self.prefix.is_empty() {
            app = axum::Router::new().nest(self.prefix, app);
        }
        self.server = Some(tokio::spawn(async move {
            let _ = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await;
        }));
    }

    /// Stop answering (the store stays open): the port refuses connections.
    pub async fn go_down(&mut self) {
        if let Some(task) = self.server.take() {
            task.abort();
            let _ = task.await;
        }
    }

    /// Answer again, on the same port.
    pub async fn come_back(&mut self) {
        let listener = TcpListener::bind(self.addr).await.expect("rebind");
        self.serve(listener);
    }

    pub fn host_port(&self) -> String {
        self.addr.to_string()
    }

    pub fn data_dir(&self) -> &Path {
        self.dir.path()
    }

    /// `POST <base><path>` with a JSON body: status and body (JSON, or the
    /// plain text as a JSON string when the body is not JSON).
    pub async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let res = client()
            .post(format!("{}{path}", self.base))
            .json(&body)
            .send()
            .await
            .expect("request");
        read(res).await
    }

    /// `POST` with raw bytes and a content type — for bodies JSON can't say.
    pub async fn post_raw(&self, path: &str, body: &str, content_type: &str) -> (u16, Value) {
        let res = client()
            .post(format!("{}{path}", self.base))
            .header("content-type", content_type)
            .body(body.to_string())
            .send()
            .await
            .expect("request");
        read(res).await
    }

    pub async fn get(&self, path: &str) -> (u16, Value) {
        let res = client()
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .expect("request");
        read(res).await
    }

    pub async fn put(&self, path: &str, body: Value) -> (u16, Value) {
        let res = client()
            .put(format!("{}{path}", self.base))
            .json(&body)
            .send()
            .await
            .expect("request");
        read(res).await
    }

    /// `POST /api/memory/<verb>`.
    pub async fn call(&self, verb: &str, body: Value) -> (u16, Value) {
        self.post(&format!("/api/memory/{verb}"), body).await
    }

    /// A verb that must succeed: its `data`.
    pub async fn ok(&self, verb: &str, body: Value) -> Value {
        let (status, env) = self.call(verb, body).await;
        assert_eq!(status, 200, "{verb}: {env}");
        assert_eq!(env["ok"], json!(true), "{verb}: {env}");
        assert_eq!(
            env.as_object().unwrap().len(),
            2,
            "{verb}: the envelope is ok + data: {env}"
        );
        env["data"].clone()
    }

    /// Add one row and return it (`data.fact`).
    pub async fn add(&self, body: Value) -> Value {
        self.ok("add", body).await["fact"].clone()
    }

    /// One MCP JSON-RPC call: the response's `result` or `error`.
    pub async fn mcp(&self, method: &str, params: Value) -> Value {
        let (status, env) = self
            .post(
                "/mcp",
                json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params }),
            )
            .await;
        assert_eq!(status, 200, "{method}: {env}");
        assert_eq!(env["jsonrpc"], "2.0");
        assert_eq!(env["id"], 7);
        env
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap()
}

async fn read(res: reqwest::Response) -> (u16, Value) {
    let status = res.status().as_u16();
    let text = res.text().await.unwrap_or_default();
    let body = serde_json::from_str(&text).unwrap_or(Value::String(text));
    (status, body)
}

/// The error envelope a refusal must be, whole: `{ok:false, error, code}`.
pub fn assert_refused(got: &(u16, Value), status: u16, code: &str, error: &str) {
    let (s, env) = got;
    assert_eq!(*s, status, "{env}");
    assert_eq!(
        env,
        &json!({ "ok": false, "error": error, "code": code }),
        "status {s}"
    );
}

/// Replace what differs between runs — ids, clock stamps — with
/// placeholders, so a whole answer can be compared with one literal.
pub fn scrub(v: &Value) -> Value {
    match v {
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), scrub_field(k, v)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(scrub).collect()),
        other => other.clone(),
    }
}

fn scrub_field(key: &str, v: &Value) -> Value {
    const STAMPS: &[&str] = &[
        "created_at",
        "updated_at",
        "expired_at",
        "resolved_at",
        "harvested_at",
        "remembered_at",
        "expires_at",
        "applied_at",
        "last_sync",
    ];
    match v {
        Value::String(_) if key == "id" => json!("<id>"),
        Value::String(_) if STAMPS.contains(&key) => json!("<time>"),
        _ => scrub(v),
    }
}

/// A stored row as an answer shows it, scrubbed: the fields every row
/// carries, plus `extra`.
pub fn row(content: &str, r#type: &str, tier: &str, from: &str, extra: Value) -> Value {
    let mut v = json!({
        "id": "<id>",
        "content": content,
        "type": r#type,
        "tier": tier,
        "from": from,
        "created_at": "<time>",
    });
    for (k, x) in extra.as_object().unwrap() {
        v[k] = x.clone();
    }
    v
}

/// A past local day, `n` days ago.
pub fn days_ago(n: i64) -> String {
    (chrono::Local::now() - chrono::Duration::days(n))
        .format("%Y-%m-%d")
        .to_string()
}

pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Local noon of `day` (`YYYY-MM-DD`) as RFC 3339 — a stamp that falls on
/// that local day in every time zone.
pub fn local_noon(day: &str) -> String {
    let d = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").unwrap();
    let local = d.and_hms_opt(12, 0, 0).unwrap();
    chrono::TimeZone::from_local_datetime(&chrono::Local, &local)
        .earliest()
        .unwrap()
        .with_timezone(&chrono::Utc)
        .to_rfc3339()
}

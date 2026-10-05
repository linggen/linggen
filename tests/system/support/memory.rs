//! A scratch ling-mem per test world: its own port, its own store inside the
//! world's `LINGGEN_HOME`, the same `serve` the real daemon runs.
//!
//! Without the embedding model (the default) it starts in about a second:
//! `session_start`, `list` and the MCP door work; an add or a search fails
//! fast (the model load is refused offline). A test that needs rows asks for
//! the model — the person's own Hugging Face cache, read through a symlink —
//! and seeds them; that costs a few seconds and ~1.2 GB, so few tests do.

use super::home::{render, Home, Vars};
use super::process::{hermetic_env, start_server, Owned};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

pub struct Memory {
    pub url: String,
    _proc: Owned,
}

/// The ling-mem binary under test: `$LING_MEM_BIN`, else the sibling
/// checkout's release build, else `ling-mem` on PATH.
pub fn ling_mem_bin() -> PathBuf {
    if let Some(p) = std::env::var_os("LING_MEM_BIN") {
        return PathBuf::from(p);
    }
    let sibling =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../linggen-memory/target/release/ling-mem");
    if sibling.is_file() {
        return sibling;
    }
    let on_path = Command::new("which").arg("ling-mem").output().ok();
    let found = on_path
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());
    match found {
        Some(p) => PathBuf::from(p),
        None => panic!(
            "system tests need a ling-mem binary: set LING_MEM_BIN, build \
             ../linggen-memory (cargo build --release), or put ling-mem on PATH"
        ),
    }
}

/// The person's Hugging Face cache, when it holds the embedding model.
fn embedding_model_cache() -> Option<PathBuf> {
    let cache = dirs::home_dir()?.join(".cache/huggingface");
    let model = cache.join("hub/models--Qwen--Qwen3-Embedding-0.6B");
    model.is_dir().then_some(cache)
}

/// Whether this machine can run the tests that embed.
pub fn embedder_available() -> bool {
    embedding_model_cache().is_some()
}

impl Memory {
    pub async fn start(home: &Home, with_embedder: bool) -> Self {
        if with_embedder {
            link_model_cache(home);
        }
        let env = hermetic_env(home);
        let spawn = |port: u16| {
            let mut cmd = Command::new(ling_mem_bin());
            cmd.arg("--data-dir").arg(home.linggen_home()).args([
                "serve",
                "--port",
                &port.to_string(),
            ]);
            Owned::spawn(cmd, &env, &home.root, home.root.join("ling-mem.log"))
        };
        let server = start_server(None, "/api/health", spawn)
            .await
            .unwrap_or_else(|e| panic!("ling-mem: {e}"));
        let url = format!("http://127.0.0.1:{}", server.port);
        Self {
            url,
            _proc: server.proc,
        }
    }

    /// POST one memory verb; the envelope's `data`, or a panic naming why.
    pub async fn call(&self, verb: &str, body: Value) -> Value {
        let resp: Value = reqwest::Client::new()
            .post(format!("{}/api/memory/{verb}", self.url))
            .json(&body)
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .unwrap_or_else(|e| panic!("ling-mem {verb}: {e}"))
            .json()
            .await
            .unwrap_or_else(|e| panic!("ling-mem {verb} body: {e}"));
        if resp["ok"] != json!(true) {
            panic!("ling-mem {verb} refused: {resp}");
        }
        resp["data"].clone()
    }

    /// Seed `tests/fixtures/memory/<file>` (placeholders rendered). Needs the
    /// embedding model.
    pub async fn seed(&self, file: &str, vars: &Vars) -> usize {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/memory")
            .join(file);
        let text = std::fs::read_to_string(&path).expect("read the memory fixture");
        let rows: Value = serde_json::from_str(&render(&text, vars)).expect("memory fixture JSON");
        let n = rows["facts"].as_array().map_or(0, Vec::len);
        self.call("add_batch", rows).await;
        n
    }
}

/// `<home>/.cache/huggingface` → the person's cache, read only (the same
/// link linggen-memory's own live-check makes).
fn link_model_cache(home: &Home) {
    let Some(real) = embedding_model_cache() else {
        panic!("no embedding model in ~/.cache/huggingface — check embedder_available() first");
    };
    let cache_dir = home.home().join(".cache");
    std::fs::create_dir_all(&cache_dir).expect("create .cache");
    std::os::unix::fs::symlink(real, cache_dir.join("huggingface")).expect("link the model cache");
}

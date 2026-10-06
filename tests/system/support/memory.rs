//! A scratch ling-mem per test world: its own port, its own store inside the
//! world's `LINGGEN_HOME`, the same `serve` the real daemon runs.
//!
//! Without the embedding model (the default) it starts in about a second:
//! `session_start`, `list` and the MCP door work; an add or a search fails
//! fast (the model load is refused offline). A test that needs rows asks for
//! the model — cloned from the person's own Hugging Face cache, never linked
//! — and seeds them; that costs a few seconds, so few tests do.

use super::home::{render, Home, Vars};
use super::process::{hermetic_env, start_server, Owned};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub struct Memory {
    pub url: String,
    _proc: Owned,
}

/// The ling-mem binary under test: `$LING_MEM_BIN`, else the sibling
/// checkout's release build, else `ling-mem` on PATH. Its path and version
/// are printed once per test process, so a stale binary shows.
pub fn ling_mem_bin() -> PathBuf {
    static BIN: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BIN.get_or_init(|| {
        let bin = find_ling_mem();
        eprintln!(
            "ling-mem under test: {} ({})",
            bin.display(),
            version_of(&bin)
        );
        bin
    })
    .clone()
}

fn find_ling_mem() -> PathBuf {
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

fn version_of(bin: &Path) -> String {
    Command::new(bin)
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|e| format!("no --version: {e}"))
}

/// The person's cached embedding model (`hub/models--…`), when it is there.
fn embedding_model_cache() -> Option<PathBuf> {
    let model = dirs::home_dir()?
        .join(".cache/huggingface/hub")
        .join(MODEL_DIR);
    model.is_dir().then_some(model)
}

/// The model's folder in a Hugging Face hub cache.
const MODEL_DIR: &str = "models--Qwen--Qwen3-Embedding-0.6B";

/// Whether this machine can run the tests that embed.
pub fn embedder_available() -> bool {
    embedding_model_cache().is_some()
}

impl Memory {
    pub async fn start(home: &Home, with_embedder: bool) -> Self {
        Self::start_on(home, with_embedder, None).await
    }

    /// Start on `port` (else a free one; a fresh one if it is taken).
    pub async fn start_on(home: &Home, with_embedder: bool, port: Option<u16>) -> Self {
        if with_embedder {
            clone_model(home);
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
        let server = start_server(port, "/api/health", spawn)
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

/// The person's cached model, cloned into `<home>/.cache/huggingface/hub`:
/// `refs/` and each snapshot file as a real file of its own (the snapshot's
/// blob resolved), so nothing ling-mem writes there — a lock, a fetch, a
/// refreshed ref — can reach the person's cache. `fs::copy` clones on APFS
/// (copy-on-write: instant, no extra space); elsewhere it is a full copy.
fn clone_model(home: &Home) {
    let Some(real) = embedding_model_cache() else {
        panic!("no embedding model in ~/.cache/huggingface — check embedder_available() first");
    };
    let ours = home.model_cache().join("hub").join(MODEL_DIR);
    clone_files(&real.join("refs"), &ours.join("refs"));
    clone_files(&real.join("snapshots"), &ours.join("snapshots"));
}

/// Copy `from` into `to`, following links: every file arrives as a real file.
fn clone_files(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create a model cache dir");
    for entry in std::fs::read_dir(from).expect("read the model cache") {
        let path = entry.expect("a model cache entry").path();
        let dest = to.join(path.file_name().expect("a file name"));
        if path.is_dir() {
            clone_files(&path, &dest);
        } else {
            let blob = path.canonicalize().expect("resolve a model file");
            std::fs::copy(&blob, &dest).expect("clone a model file");
        }
    }
}

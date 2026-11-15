//! The world's memory: the one the engine under test hosts in its own
//! process, over the world's `LINGGEN_HOME` store, at `/mem` on the engine's
//! port (`memory_host`). No second process.
//!
//! Without the embedding model (the default) `session_start`, `list` and the
//! MCP door work; an add or a search fails fast (the model load is refused
//! offline). A test that needs rows asks for the model — cloned from the
//! person's own Hugging Face cache, never linked — and seeds them once the
//! engine is up; that costs a few seconds, so few tests do.

use super::home::{render, Home, Vars};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct Memory {
    pub url: String,
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

/// The address of the memory an engine on `port` hosts.
pub fn url_on(port: u16) -> String {
    format!("http://127.0.0.1:{port}/mem")
}

/// Whether this machine can run the tests that embed.
pub fn embedder_available() -> bool {
    embedding_model_cache().is_some()
}

impl Memory {
    /// Get the world's home ready for its memory (the model, when asked for)
    /// before the engine starts.
    pub fn prepare(home: &Home, with_embedder: bool) {
        if with_embedder {
            clone_model(home);
        }
    }

    /// The memory the engine on `port` hosts.
    pub fn hosted(port: u16) -> Self {
        Self { url: url_on(port) }
    }

    /// POST one memory verb; the envelope's `data`, or a panic naming why.
    pub async fn call(&self, verb: &str, body: Value) -> Value {
        let resp: Value = reqwest::Client::new()
            .post(format!("{}/api/memory/{verb}", self.url))
            .json(&body)
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .unwrap_or_else(|e| panic!("memory {verb}: {e}"))
            .json()
            .await
            .unwrap_or_else(|e| panic!("memory {verb} body: {e}"));
        if resp["ok"] != json!(true) {
            panic!("memory {verb} refused: {resp}");
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
/// blob resolved), so nothing memory writes there — a lock, a fetch, a
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

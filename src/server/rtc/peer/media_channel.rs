//! Binary media channel — bulk bytes over WebRTC, off the control channel.
//!
//! The `http_request` RPC on the control channel is a JSON envelope: bodies are
//! decoded as UTF-8, gzipped and base64'd. That is fine for text and hopeless
//! for photos — measured at ~0.4 MB/s, and binary arrives corrupted because the
//! UTF-8 decode is lossy. This channel carries the bytes themselves.
//!
//! One transfer at a time per channel, framed by text messages around the
//! binary chunks:
//!
//! ```text
//! → {"type":"put_begin","id","local_id","name","size","sha256","created_ms"}
//! → <binary chunk> …                      (raw file bytes, in order)
//! → {"type":"put_end","id"}
//! ← {"type":"put_ok","id","bytes"} | {"type":"put_err","id","error"}
//! ```
//!
//! Completed transfers land through the same `finalize_ingest` the HTTP upload
//! uses, so archiving, the ledger, and the scan trigger behave identically no
//! matter which path carried the file.

use std::path::PathBuf;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

/// A transfer in flight on one peer's media channel.
pub(super) struct MediaTransfer {
    id: String,
    local_id: String,
    name: String,
    declared_sha: String,
    declared_size: u64,
    created_ms: Option<i64>,
    tmp: PathBuf,
    file: tokio::fs::File,
    hasher: Sha256,
    received: u64,
}

impl MediaTransfer {
    /// Bytes written so far — the phone's progress is our progress.
    pub(super) fn received(&self) -> u64 {
        self.received
    }
}

/// Handle a text control frame. Returns a reply to send back, and possibly a
/// new in-flight transfer to hold.
pub(super) async fn handle_text(
    text: &str,
    current: &mut Option<MediaTransfer>,
) -> Option<String> {
    let msg: Value = serde_json::from_str(text).ok()?;
    match msg.get("type").and_then(Value::as_str)? {
        "put_begin" => {
            // A new transfer supersedes anything half-received: the phone only
            // starts one after the previous ack, so this means it gave up.
            if let Some(prev) = current.take() {
                let _ = tokio::fs::remove_file(&prev.tmp).await;
            }
            match begin(&msg).await {
                Ok(t) => {
                    *current = Some(t);
                    None
                }
                Err(e) => Some(
                    json!({
                        "type": "put_err",
                        "id": msg.get("id").cloned().unwrap_or(Value::Null),
                        "error": e.to_string(),
                    })
                    .to_string(),
                ),
            }
        }
        "put_end" => {
            let t = current.take()?;
            Some(finish(t).await)
        }
        _ => None,
    }
}

/// Handle one binary chunk. Returns an error reply if the transfer must abort.
pub(super) async fn handle_binary(
    bytes: &[u8],
    current: &mut Option<MediaTransfer>,
) -> Option<String> {
    let t = current.as_mut()?;
    if t.received + bytes.len() as u64 > t.declared_size {
        let failed = current.take()?;
        let _ = tokio::fs::remove_file(&failed.tmp).await;
        return Some(
            json!({ "type": "put_err", "id": failed.id, "error": "more bytes than declared" })
                .to_string(),
        );
    }
    if let Err(e) = t.file.write_all(bytes).await {
        let failed = current.take()?;
        let _ = tokio::fs::remove_file(&failed.tmp).await;
        return Some(
            json!({ "type": "put_err", "id": failed.id, "error": format!("write: {e}") })
                .to_string(),
        );
    }
    t.hasher.update(bytes);
    t.received += bytes.len() as u64;
    None
}

/// Drop a half-received transfer when the peer goes away.
pub(super) async fn abandon(current: &mut Option<MediaTransfer>) {
    if let Some(t) = current.take() {
        let _ = tokio::fs::remove_file(&t.tmp).await;
    }
}

async fn begin(msg: &Value) -> anyhow::Result<MediaTransfer> {
    let field = |k: &str| -> anyhow::Result<String> {
        msg.get(k)
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("missing {k}"))
    };
    let id = field("id")?;
    let local_id = field("local_id")?;
    let declared_sha = field("sha256")?.to_lowercase();
    let declared_size = msg
        .get("size")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("missing size"))?;
    let name = msg
        .get("name")
        .and_then(Value::as_str)
        .map(crate::server::api::media::sanitize_filename)
        .unwrap_or_else(|| format!("{}.bin", &declared_sha[..12.min(declared_sha.len())]));

    let staging = crate::server::api::media::staging_dir();
    tokio::fs::create_dir_all(&staging).await?;
    let tmp = staging.join(format!(".rtc-{id}-{}.part", std::process::id()));
    let file = tokio::fs::File::create(&tmp).await?;

    Ok(MediaTransfer {
        id,
        local_id,
        name,
        declared_sha,
        declared_size,
        created_ms: msg.get("created_ms").and_then(Value::as_i64),
        tmp,
        file,
        hasher: Sha256::new(),
        received: 0,
    })
}

async fn finish(mut t: MediaTransfer) -> String {
    let reply_err = |id: &str, e: String| json!({"type": "put_err", "id": id, "error": e}).to_string();

    if let Err(e) = t.file.flush().await {
        let _ = tokio::fs::remove_file(&t.tmp).await;
        return reply_err(&t.id, format!("flush: {e}"));
    }
    drop(t.file);

    if t.received != t.declared_size {
        let _ = tokio::fs::remove_file(&t.tmp).await;
        return reply_err(
            &t.id,
            format!("size mismatch: declared {}, got {}", t.declared_size, t.received),
        );
    }
    let computed = format!("{:x}", t.hasher.finalize());
    if computed != t.declared_sha {
        let _ = tokio::fs::remove_file(&t.tmp).await;
        return reply_err(&t.id, "sha256 mismatch".to_string());
    }

    // Same landing as the HTTP upload — one definition of "ingested".
    let (local_id, name, tmp, size, created_ms) =
        (t.local_id.clone(), t.name.clone(), t.tmp.clone(), t.received, t.created_ms);
    let sha = computed.clone();
    let placed = tokio::task::spawn_blocking(move || {
        crate::server::api::media::finalize_ingest(&local_id, &sha, created_ms, &name, &tmp, size)
    })
    .await;

    match placed {
        Ok(Ok(())) => {
            crate::server::api::media::schedule_wireless_scan();
            json!({ "type": "put_ok", "id": t.id, "bytes": t.received }).to_string()
        }
        Ok(Err(e)) => reply_err(&t.id, e.to_string()),
        Err(e) => reply_err(&t.id, format!("ingest task: {e}")),
    }
}

//! Response enqueue + large-body gzip+base64 chunking for control/session channels.
//!
//! Writes never go directly to str0m channels — SCTP requires a poll_output()
//! between writes, so all responses go through a single pending-writes queue
//! the main loop drains. Large bodies are compressed and split into multiple
//! JSON messages so they fit under the per-message SCTP limit.

use std::collections::VecDeque;

/// Max base64 chunk size per JSON message (~48 KB raw → ~64 KB base64, well under 256 KB SCTP limit).
pub(super) const MAX_CHUNK_RAW: usize = 48_000;
/// Max pending data channel writes before dropping new messages.
pub(super) const MAX_DC_WRITE_QUEUE: usize = 4000;

/// Enqueue a response, using gzip + base64 for large bodies.
///
/// Protocol for large responses (all text, no binary DC):
/// 1. JSON: `{ request_id, gzip_start: { total_bytes, chunks, status } }`
/// 2. JSON: `{ request_id, gzip_chunk: "<base64 data>" }` × N
/// 3. JSON: `{ request_id, gzip_end: true }`
///
/// Client collects base64 chunks, decodes to binary, decompresses gzip.
pub(super) fn enqueue_response(
    queue: &mut VecDeque<(str0m::channel::ChannelId, String)>,
    cid: str0m::channel::ChannelId,
    request_id: &str,
    result: serde_json::Value,
) {
    if queue.len() >= MAX_DC_WRITE_QUEUE {
        tracing::warn!(
            "DC write queue full ({MAX_DC_WRITE_QUEUE}), dropping response for {request_id}"
        );
        return;
    }

    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;

    let body = result
        .get("data")
        .and_then(|d| d.get("body"))
        .and_then(|b| b.as_str());

    if let Some(body_str) = body {
        if body_str.len() > MAX_CHUNK_RAW {
            let status = result
                .get("data")
                .and_then(|d| d.get("status"))
                .and_then(|s| s.as_u64())
                .unwrap_or(200);

            // Gzip compress the body
            use flate2::write::GzEncoder;
            use flate2::Compression;
            use std::io::Write;
            let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
            encoder.write_all(body_str.as_bytes()).ok();
            let compressed = encoder.finish().unwrap_or_default();

            // Split compressed bytes into chunks, base64-encode each
            let raw_chunks: Vec<&[u8]> = compressed.chunks(MAX_CHUNK_RAW).collect();
            let num_chunks = raw_chunks.len();

            tracing::debug!(
                "Response for {request_id}: {}KB → gzip {}KB → {num_chunks} base64 chunks",
                body_str.len() / 1024,
                compressed.len() / 1024,
            );

            // Check if there's enough room for the full gzip transfer (header + chunks + footer)
            let needed = 2 + num_chunks;
            if queue.len() + needed > MAX_DC_WRITE_QUEUE {
                tracing::warn!(
                    "DC write queue too full for gzip response ({needed} entries needed), dropping {request_id}"
                );
                let err = serde_json::json!({ "request_id": request_id, "error": "Queue full" });
                queue.push_back((cid, err.to_string()));
                return;
            }

            // Header
            let header = serde_json::json!({
                "request_id": request_id,
                "gzip_start": { "total_bytes": compressed.len(), "chunks": num_chunks, "status": status }
            });
            queue.push_back((cid, header.to_string()));

            // Base64-encoded chunks
            for chunk in &raw_chunks {
                let encoded = b64.encode(chunk);
                let msg = serde_json::json!({
                    "request_id": request_id,
                    "gzip_chunk": encoded
                });
                queue.push_back((cid, msg.to_string()));
            }

            // Footer
            let footer = serde_json::json!({
                "request_id": request_id,
                "gzip_end": true
            });
            queue.push_back((cid, footer.to_string()));
            return;
        }
    }

    // Small response — single JSON message
    let mut resp = result;
    resp["request_id"] = serde_json::Value::String(request_id.to_string());
    queue.push_back((cid, resp.to_string()));
}

/// Enqueue an unsolicited push (no request_id), e.g. `page_state`.
///
/// Unlike RPC responses, pushes have no `data.body` string to key on, so the
/// whole serialized message is the payload. Small messages go out as one JSON
/// frame; large ones (e.g. a `page_state` carrying a long `all_sessions` list)
/// are gzip-compressed and split into base64 chunks so each frame stays under
/// the ~64 KB SCTP per-message limit. Without this, an oversized single frame
/// makes the peer reset the data channel (DTLS CloseNotify) and the client
/// never receives the state.
///
/// Wire protocol (mirrors the response gzip path, but unkeyed):
/// 1. `{ push_gzip_start: { total_bytes, chunks } }`
/// 2. `{ push_gzip_chunk: "<base64>" }` × N
/// 3. `{ push_gzip_end: true }`
pub(super) fn enqueue_push(
    queue: &mut VecDeque<(str0m::channel::ChannelId, String)>,
    cid: str0m::channel::ChannelId,
    msg: serde_json::Value,
) {
    let serialized = msg.to_string();
    if serialized.len() <= MAX_CHUNK_RAW {
        if queue.len() < MAX_DC_WRITE_QUEUE {
            queue.push_back((cid, serialized));
        }
        return;
    }

    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;

    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(serialized.as_bytes()).ok();
    let compressed = encoder.finish().unwrap_or_default();

    let raw_chunks: Vec<&[u8]> = compressed.chunks(MAX_CHUNK_RAW).collect();
    let num_chunks = raw_chunks.len();

    // Header + chunks + footer.
    let needed = 2 + num_chunks;
    if queue.len() + needed > MAX_DC_WRITE_QUEUE {
        tracing::warn!("DC write queue too full for gzip push ({needed} entries needed), dropping");
        return;
    }

    tracing::debug!(
        "Push: {}KB → gzip {}KB → {num_chunks} base64 chunks",
        serialized.len() / 1024,
        compressed.len() / 1024,
    );

    let header = serde_json::json!({
        "push_gzip_start": { "total_bytes": compressed.len(), "chunks": num_chunks }
    });
    queue.push_back((cid, header.to_string()));

    for chunk in &raw_chunks {
        let msg = serde_json::json!({ "push_gzip_chunk": b64.encode(chunk) });
        queue.push_back((cid, msg.to_string()));
    }

    let footer = serde_json::json!({ "push_gzip_end": true });
    queue.push_back((cid, footer.to_string()));
}

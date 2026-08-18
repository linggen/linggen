//! Client → server gzip+base64 reassembly for large control messages.
//!
//! The server has always chunked its own large responses (see `response.rs`);
//! this is the same protocol in the other direction. It exists because a
//! single SCTP message is capped — str0m advertises `LOCAL_MAX_MESSAGE_SIZE`
//! (256 KB) and browsers refuse an oversized `send()` by tearing the channel
//! down, which surfaces to the user as a dropped connection rather than as
//! "too big". A pasted image is the payload that hits it: base64 inflates a
//! file by 4/3, so anything over a ~190 KB source overflowed and killed the
//! chat.
//!
//! Protocol, mirroring `response.rs` with a `req_` prefix so the two
//! directions can never be confused on the wire:
//! 1. `{ transfer_id, req_gzip_start: { chunks } }`
//! 2. `{ transfer_id, req_gzip_chunk: "<base64>" }` × N
//! 3. `{ transfer_id, req_gzip_end: true }`
//!
//! The reassembled bytes are one gzipped JSON control message, handled from
//! `req_gzip_end` exactly as if it had arrived whole.

use std::collections::HashMap;

/// Cap on a single reassembled control message. Generous next to the 256 KB
/// per-SCTP-message limit this works around, but not unbounded: chunks are
/// buffered in memory before the message is parsed, so an unauthenticated
/// peer must not be able to make the daemon hold arbitrarily much.
const MAX_INBOUND_BYTES: usize = 32 * 1024 * 1024;

/// Max concurrent inbound transfers per peer.
const MAX_INBOUND_TRANSFERS: usize = 8;

#[derive(Default)]
pub(super) struct InboundReassembly {
    transfers: HashMap<String, Transfer>,
}

struct Transfer {
    expected: usize,
    chunks: Vec<String>,
    bytes: usize,
}

/// What `handle_inbound_chunk` decided about a message.
pub(super) enum Inbound {
    /// Not part of a chunked transfer — handle it normally.
    NotChunked,
    /// Part of a transfer still in flight; nothing to do yet.
    Buffered,
    /// A transfer completed — this is the reassembled control message.
    Complete(String),
    /// The transfer failed; the string is a reason worth logging.
    Failed(String),
}

impl InboundReassembly {
    /// Feed one control-channel message. Returns `NotChunked` for anything
    /// that is not part of a transfer, so callers fall through to the normal
    /// path untouched.
    pub(super) fn accept(&mut self, msg: &serde_json::Value) -> Inbound {
        let Some(transfer_id) = msg.get("transfer_id").and_then(|v| v.as_str()) else {
            return Inbound::NotChunked;
        };

        if let Some(start) = msg.get("req_gzip_start") {
            if self.transfers.len() >= MAX_INBOUND_TRANSFERS {
                self.transfers.clear();
                return Inbound::Failed(format!(
                    "too many concurrent inbound transfers (>{MAX_INBOUND_TRANSFERS}), dropped all"
                ));
            }
            let expected = start.get("chunks").and_then(|c| c.as_u64()).unwrap_or(0) as usize;
            if expected == 0 {
                return Inbound::Failed("req_gzip_start declared 0 chunks".to_string());
            }
            self.transfers.insert(
                transfer_id.to_string(),
                Transfer { expected, chunks: Vec::with_capacity(expected), bytes: 0 },
            );
            return Inbound::Buffered;
        }

        if let Some(chunk) = msg.get("req_gzip_chunk").and_then(|v| v.as_str()) {
            let Some(t) = self.transfers.get_mut(transfer_id) else {
                return Inbound::Failed(format!("chunk for unknown transfer {transfer_id}"));
            };
            t.bytes += chunk.len();
            if t.bytes > MAX_INBOUND_BYTES || t.chunks.len() >= t.expected {
                self.transfers.remove(transfer_id);
                return Inbound::Failed(format!(
                    "transfer {transfer_id} exceeded its declared size"
                ));
            }
            t.chunks.push(chunk.to_string());
            return Inbound::Buffered;
        }

        if msg.get("req_gzip_end").is_some() {
            let Some(t) = self.transfers.remove(transfer_id) else {
                return Inbound::Failed(format!("end for unknown transfer {transfer_id}"));
            };
            if t.chunks.len() != t.expected {
                return Inbound::Failed(format!(
                    "transfer {transfer_id} ended with {}/{} chunks",
                    t.chunks.len(),
                    t.expected
                ));
            }
            return match inflate(&t.chunks) {
                Ok(text) => Inbound::Complete(text),
                Err(e) => Inbound::Failed(format!("transfer {transfer_id}: {e}")),
            };
        }

        Inbound::NotChunked
    }

    /// Drop everything buffered — called when the channel goes away, so a
    /// half-sent transfer can't outlive the connection that started it.
    pub(super) fn clear(&mut self) {
        self.transfers.clear();
    }
}

/// base64 chunks → concatenated bytes → gunzip → UTF-8.
fn inflate(chunks: &[String]) -> Result<String, String> {
    use base64::Engine;
    use std::io::Read;

    let b64 = base64::engine::general_purpose::STANDARD;
    let mut compressed = Vec::new();
    for chunk in chunks {
        let decoded = b64.decode(chunk).map_err(|e| format!("base64: {e}"))?;
        compressed.extend_from_slice(&decoded);
    }

    let mut decoder = flate2::read::GzDecoder::new(&compressed[..]);
    let mut out = String::new();
    decoder
        .read_to_string(&mut out)
        .map_err(|e| format!("gzip: {e}"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a message the way the client does, so the test exercises the
    /// real wire format rather than a hand-built approximation.
    fn encode(transfer_id: &str, text: &str, chunk_size: usize) -> Vec<serde_json::Value> {
        use base64::Engine;
        use std::io::Write;
        let b64 = base64::engine::general_purpose::STANDARD;

        let mut encoder =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(text.as_bytes()).unwrap();
        let compressed = encoder.finish().unwrap();
        let chunks: Vec<&[u8]> = compressed.chunks(chunk_size).collect();

        let mut out = vec![serde_json::json!({
            "transfer_id": transfer_id,
            "req_gzip_start": { "chunks": chunks.len() }
        })];
        for c in &chunks {
            out.push(serde_json::json!({
                "transfer_id": transfer_id,
                "req_gzip_chunk": b64.encode(c)
            }));
        }
        out.push(serde_json::json!({ "transfer_id": transfer_id, "req_gzip_end": true }));
        out
    }

    fn feed(r: &mut InboundReassembly, msgs: Vec<serde_json::Value>) -> Inbound {
        let mut last = Inbound::NotChunked;
        for m in msgs {
            last = r.accept(&m);
        }
        last
    }

    #[test]
    fn reassembles_a_multi_chunk_message() {
        // A chat request carrying an image far past the 256 KB SCTP ceiling.
        let big = "A".repeat(600_000);
        let original = serde_json::json!({
            "type": "chat", "request_id": "req-1", "message": "hi", "images": [big]
        })
        .to_string();

        let mut r = InboundReassembly::default();
        match feed(&mut r, encode("t1", &original, 48_000)) {
            Inbound::Complete(text) => assert_eq!(text, original),
            _ => panic!("expected Complete"),
        }
        assert!(r.transfers.is_empty(), "completed transfer must not leak");
    }

    #[test]
    fn single_chunk_round_trips() {
        let original = r#"{"type":"chat","message":"small"}"#;
        let mut r = InboundReassembly::default();
        match feed(&mut r, encode("t1", original, 48_000)) {
            Inbound::Complete(text) => assert_eq!(text, original),
            _ => panic!("expected Complete"),
        }
    }

    #[test]
    fn interleaved_transfers_do_not_mix() {
        let a = r#"{"type":"chat","message":"aaa"}"#;
        let b = r#"{"type":"chat","message":"bbb"}"#;
        let (ma, mb) = (encode("ta", a, 8), encode("tb", b, 8));

        let mut r = InboundReassembly::default();
        // Interleave: starts, then all of B, then the rest of A.
        r.accept(&ma[0]);
        r.accept(&mb[0]);
        for m in &mb[1..] {
            if let Inbound::Complete(text) = r.accept(m) {
                assert_eq!(text, b);
            }
        }
        match feed(&mut r, ma[1..].to_vec()) {
            Inbound::Complete(text) => assert_eq!(text, a),
            _ => panic!("expected Complete for A"),
        }
    }

    #[test]
    fn plain_messages_pass_through() {
        let mut r = InboundReassembly::default();
        let plain = serde_json::json!({ "type": "heartbeat", "ts": 1 });
        assert!(matches!(r.accept(&plain), Inbound::NotChunked));
    }

    #[test]
    fn rejects_more_chunks_than_declared() {
        let mut msgs = encode("t1", "hello", 8);
        let extra = msgs[1].clone();
        msgs.insert(2, extra); // one chunk too many
        let mut r = InboundReassembly::default();
        assert!(matches!(feed(&mut r, msgs), Inbound::Failed(_)));
        assert!(r.transfers.is_empty(), "failed transfer must be dropped");
    }

    #[test]
    fn rejects_truncated_transfer() {
        let msgs = encode("t1", &"x".repeat(100_000), 8);
        let mut r = InboundReassembly::default();
        // Everything except the last chunk, then the end marker.
        for m in &msgs[..msgs.len() - 2] {
            r.accept(m);
        }
        let end = msgs.last().unwrap().clone();
        assert!(matches!(r.accept(&end), Inbound::Failed(_)));
    }

    #[test]
    fn chunk_for_unknown_transfer_is_rejected() {
        let mut r = InboundReassembly::default();
        let orphan = serde_json::json!({ "transfer_id": "nope", "req_gzip_chunk": "AAAA" });
        assert!(matches!(r.accept(&orphan), Inbound::Failed(_)));
    }

    #[test]
    fn clear_drops_partial_transfers() {
        let msgs = encode("t1", &"y".repeat(100_000), 8);
        let mut r = InboundReassembly::default();
        for m in &msgs[..3] {
            r.accept(m);
        }
        assert!(!r.transfers.is_empty());
        r.clear();
        assert!(r.transfers.is_empty());
    }
}

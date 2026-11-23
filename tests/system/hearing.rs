//! Her hearing, through the real route (`/api/stt`): the router, the body
//! limit and the handlers, as a page's request reaches them. The world is a
//! machine with 8 GB of memory, which the voice lane's gate refuses: it
//! cannot hear, and no model fetch can start in a test. What is heard from a
//! clip — the words, the owner's door, the GET beside the POST — is
//! `api/stt.rs`'s unit tests, with a fake hearing engine.

use crate::support::{Setup, World};
use base64::Engine as _;
use serde_json::json;

/// A 16 kHz mono 16-bit WAV of `secs` seconds of silence.
fn silence(secs: usize) -> Vec<u8> {
    let data_len = (16_000 * 2 * secs) as u32;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&16_000u32.to_le_bytes());
    wav.extend_from_slice(&32_000u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.resize(44 + data_len as usize, 0);
    wav
}

const NO_MEMORY: &str = "Hearing needs 16 GB of memory, this Mac has 8 GB.";

async fn deaf_world() -> World {
    World::start(Setup { machine_memory_gb: Some(8), ..Setup::default().setting_off() }).await
}

/// Rule 1: whether she hears is asked before a microphone opens, and a
/// machine that cannot says why in words.
#[tokio::test]
async fn a_machine_that_cannot_hear_says_so_before_any_clip() {
    let w = deaf_world().await;
    let said = w.api.get("/api/stt").await;
    assert_eq!(said, json!({ "hears": false, "reason": NO_MEMORY, "fetching": false, "next": "never" }));
}

/// The POST says the same, silence or not: the reason, never "nothing heard".
#[tokio::test]
async fn a_silent_clip_on_a_machine_that_cannot_hear_gets_the_reason() {
    let w = deaf_world().await;
    let (status, body) = w.api.post_bytes("/api/stt", "audio/wav", silence(2)).await;
    assert_eq!(status, 503);
    assert_eq!(body, json!({ "error": "unavailable", "reason": NO_MEMORY, "fetching": false, "next": "never" }));
}

/// A minute of audio as JSON is over the framework's body limit; this route
/// reads it, which is what lets a page's data-channel request carry a clip.
/// (Read, and then judged: 503, not the framework's 413.)
#[tokio::test]
async fn a_minute_of_audio_in_the_json_form_is_read() {
    let w = deaf_world().await;
    let b64 = base64::engine::general_purpose::STANDARD.encode(silence(60));
    assert!(b64.len() > 2 * 1024 * 1024, "the clip is over the default body limit");
    let (status, body) = w.api.post_with_status("/api/stt", json!({ "wav_b64": b64 })).await;
    assert_eq!((status, body["error"].as_str()), (503, Some("unavailable")));
}

/// Only the two body forms are taken: any other is refused before a hearing
/// or a fetch can start, whatever the machine.
#[tokio::test]
async fn a_body_that_is_neither_wav_nor_json_is_refused_whatever_the_machine() {
    let w = deaf_world().await;
    for content_type in ["text/plain", "application/octet-stream", "audio/mpeg"] {
        let (status, body) = w.api.post_bytes("/api/stt", content_type, silence(2)).await;
        assert_eq!((status, body["error"].as_str()), (415, Some("unsupported_media_type")), "{content_type}");
    }
}

/// Whether a line can be spoken is asked the way hearing is: a read that says
/// which voice a line would have, and starts nothing (no fetch, no model load).
#[tokio::test]
async fn whether_she_speaks_is_asked_without_starting_anything() {
    let w = deaf_world().await;
    let said = w.api.get("/api/tts").await;
    assert!(said["speaks"].is_boolean() && said["natural"].is_boolean(), "{said}");
    assert!(said.get("stub").is_none());
    if said["speaks"] == json!(false) {
        assert!(said["reason"].as_str().is_some_and(|r| !r.is_empty()), "a no gives its reason: {said}");
    }
}

//! Her hearing, through the real route (`POST /api/stt`): the router, the
//! body limit and the handler, as a page's request reaches them. Only silent
//! or invalid clips are sent here: a clip with speech in it would reach the
//! machine's hearing, and a world must never start that model's download.
//! What is heard from a clip — the words, the owner's door, the 503 — is
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

#[tokio::test]
async fn a_silent_clip_is_empty_text() {
    let w = World::start(Setup::default().setting_off()).await;
    let (status, body) = w.api.post_bytes("/api/stt", "audio/wav", silence(2)).await;
    assert_eq!((status, body), (200, json!({ "text": "" })));
}

/// A minute of audio as JSON is over the framework's body limit; this route
/// reads it, which is what lets a page's data-channel request carry a clip.
#[tokio::test]
async fn a_minute_of_audio_in_the_json_form_is_read() {
    let w = World::start(Setup::default().setting_off()).await;
    let b64 = base64::engine::general_purpose::STANDARD.encode(silence(60));
    assert!(b64.len() > 2 * 1024 * 1024, "the clip is over the default body limit");
    let (status, body) = w.api.post_with_status("/api/stt", json!({ "wav_b64": b64 })).await;
    assert_eq!((status, body), (200, json!({ "text": "" })));
}

#[tokio::test]
async fn a_body_that_is_not_that_wav_is_refused() {
    let w = World::start(Setup::default().setting_off()).await;
    let (status, body) = w.api.post_bytes("/api/stt", "audio/wav", b"not a clip".to_vec()).await;
    assert_eq!(status, 400);
    assert_eq!(body["error"], "bad_audio");
    let (status, body) = w.api.post_bytes("/api/stt", "audio/wav", silence(61)).await;
    assert_eq!((status, body["error"].as_str()), (413, Some("too_long")));
}

//! A world's voice: a stand-in for macOS `say` on the world's `PATH`.
//!
//! The engine's speech falls back to `say` when its own voice models are not
//! there — and in a world they never are (no runtime, no weights, offline).
//! The stand-in writes one fixed clip, so a test that makes Yinyue speak
//! gets real WAV bytes, the same on every machine, with no audio device and
//! no `say` at all (Linux).

use std::path::Path;

/// The stand-in clip's length, seconds — what a test reads back once the
/// page has decoded it (all of it arrived, or the length is off).
pub const CLIP_SECS: f64 = 1.5;
const RATE: u32 = 22_050;

/// `<root>/bin/say` and the clip it hands out, `<root>/bin/voice.wav`.
pub fn install(root: &Path) {
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).expect("create the world's bin");
    let clip = bin.join("voice.wav");
    std::fs::write(&clip, tone_wav()).expect("write the stand-in clip");
    let say = bin.join("say");
    std::fs::write(&say, say_script(&clip)).expect("write the say stand-in");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&say, std::fs::Permissions::from_mode(0o755))
        .expect("make the say stand-in executable");
}

/// `say … -o <out> …` → the clip, copied to `<out>`.
fn say_script(clip: &Path) -> String {
    format!(
        "#!/bin/sh\n\
         # A world's stand-in for `say` (tests/system/support/voice.rs).\n\
         while [ $# -gt 0 ]; do\n  \
           if [ \"$1\" = -o ]; then out=\"$2\"; fi\n  \
           shift\n\
         done\n\
         [ -n \"$out\" ] && cp '{}' \"$out\"\n",
        clip.display()
    )
}

/// A 440 Hz tone, 16-bit mono PCM WAVE at 22.05 kHz — `say`'s format here.
fn tone_wav() -> Vec<u8> {
    let samples = (RATE as f64 * CLIP_SECS) as u32;
    let data_len = samples * 2;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..samples {
        let t = i as f64 / RATE as f64;
        let s = (t * 440.0 * std::f64::consts::TAU).sin() * 8000.0;
        wav.extend_from_slice(&(s as i16).to_le_bytes());
    }
    wav
}

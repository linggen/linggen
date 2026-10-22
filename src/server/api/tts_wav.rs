//! Just enough WAV for the voice: read a clip's samples, measure how much of
//! it is voiced, and join clauses with silences into one clip again.

/// Mono samples at one rate.
#[derive(Debug, Clone, PartialEq)]
pub struct Pcm {
    pub rate: u32,
    pub samples: Vec<i16>,
}

/// Loudness frames for [`voiced_secs`]: 20 ms each, voiced when within
/// 35 dB of the clip's loudest frame.
const FRAME_SECS: f32 = 0.02;
const VOICED_DB: f32 = -35.0;

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

struct Format {
    tag: u16,
    channels: u16,
    rate: u32,
    bits: u16,
}

/// Decode a RIFF/WAVE clip (PCM16 or float32, any channel count — the
/// first channel is kept). Walks the chunks; never assumes a 44-byte header.
pub fn decode(bytes: &[u8]) -> anyhow::Result<Pcm> {
    anyhow::ensure!(
        bytes.get(0..4) == Some(b"RIFF") && bytes.get(8..12) == Some(b"WAVE"),
        "not a WAV clip"
    );
    let mut fmt = None;
    let mut at = 12;
    while let (Some(id), Some(len)) = (bytes.get(at..at + 4), u32_at(bytes, at + 4)) {
        let body = at + 8;
        let end = (body + len as usize).min(bytes.len());
        match id {
            b"fmt " => fmt = read_format(bytes, body),
            b"data" => {
                let f = fmt.ok_or_else(|| anyhow::anyhow!("WAV data before fmt"))?;
                return samples(&bytes[body..end], &f);
            }
            _ => {}
        }
        at = body + len as usize + (len as usize & 1);
    }
    anyhow::bail!("WAV has no data chunk")
}

fn read_format(b: &[u8], at: usize) -> Option<Format> {
    Some(Format {
        tag: u16_at(b, at)?,
        channels: u16_at(b, at + 2)?.max(1),
        rate: u32_at(b, at + 4)?,
        bits: u16_at(b, at + 14)?,
    })
}

fn samples(data: &[u8], f: &Format) -> anyhow::Result<Pcm> {
    let width = (f.bits / 8) as usize;
    let stride = width * f.channels as usize;
    let read: fn(&[u8]) -> i16 = match (f.tag, f.bits) {
        (1, 16) => pcm16,
        (3, 32) => float32,
        (tag, bits) => anyhow::bail!("unsupported WAV format {tag}/{bits}-bit"),
    };
    Ok(Pcm {
        rate: f.rate,
        samples: data.chunks_exact(stride).map(read).collect(),
    })
}

fn pcm16(s: &[u8]) -> i16 {
    i16::from_le_bytes([s[0], s[1]])
}

fn float32(s: &[u8]) -> i16 {
    let v = f32::from_le_bytes([s[0], s[1], s[2], s[3]]);
    (v.clamp(-1.0, 1.0) * 32767.0) as i16
}

/// Encode mono PCM16.
pub fn encode(pcm: &Pcm) -> Vec<u8> {
    let data_len = (pcm.samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&pcm.rate.to_le_bytes());
    out.extend_from_slice(&(pcm.rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in &pcm.samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Seconds of the clip that are voiced — silence at the edges and in the
/// gaps does not count, so a clause's rate is its speech rate.
pub fn voiced_secs(pcm: &Pcm) -> f32 {
    let frame = ((pcm.rate as f32 * FRAME_SECS) as usize).max(1);
    let rms: Vec<f32> = pcm
        .samples
        .chunks_exact(frame)
        .map(|f| {
            let sum: f32 = f.iter().map(|&s| (s as f32 / 32768.0).powi(2)).sum();
            (sum / frame as f32).sqrt() + 1e-9
        })
        .collect();
    let loudest = rms.iter().copied().fold(1e-9, f32::max);
    let floor = loudest * 10f32.powf(VOICED_DB / 20.0);
    rms.iter().filter(|&&r| r > floor).count() as f32 * FRAME_SECS
}

/// Linear resample to `rate` (only a fallback clip from another engine
/// ever needs it; both voices run at 24 kHz today).
fn resample(pcm: &Pcm, rate: u32) -> Vec<i16> {
    if pcm.rate == rate || pcm.samples.is_empty() {
        return pcm.samples.clone();
    }
    let ratio = pcm.rate as f64 / rate as f64;
    let n = (pcm.samples.len() as f64 / ratio) as usize;
    let last = pcm.samples.len() - 1;
    (0..n)
        .map(|i| {
            let x = i as f64 * ratio;
            let (j, t) = (x.floor() as usize, x.fract());
            let (a, b) = (pcm.samples[j.min(last)], pcm.samples[(j + 1).min(last)]);
            (a as f64 + (b as f64 - a as f64) * t) as i16
        })
        .collect()
}

/// Join clips, each followed by its pause in ms, at the first clip's rate.
pub fn join(parts: &[(Pcm, u32)]) -> Pcm {
    let rate = parts.first().map_or(24_000, |(p, _)| p.rate);
    let mut samples = Vec::new();
    for (pcm, pause_ms) in parts {
        samples.extend(resample(pcm, rate));
        samples.extend(std::iter::repeat_n(0i16, (rate * pause_ms / 1000) as usize));
    }
    Pcm { rate, samples }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, voiced: f32, silent: f32) -> Pcm {
        let on = (rate as f32 * voiced) as usize;
        let off = (rate as f32 * silent) as usize;
        let mut samples: Vec<i16> = (0..on)
            .map(|i| ((i as f32 * 0.3).sin() * 8000.0) as i16)
            .collect();
        samples.extend(std::iter::repeat_n(0, off));
        Pcm { rate, samples }
    }

    #[test]
    fn encode_then_decode_round_trips() {
        let pcm = tone(24_000, 0.1, 0.05);
        assert_eq!(decode(&encode(&pcm)).unwrap(), pcm);
    }

    #[test]
    fn decode_skips_unknown_chunks_and_reads_float() {
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF\0\0\0\0WAVE");
        b.extend_from_slice(b"LIST");
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(b"abc\0"); // odd length + pad byte
        b.extend_from_slice(b"fmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        for v in [3u16, 1] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&24_000u32.to_le_bytes());
        b.extend_from_slice(&96_000u32.to_le_bytes());
        b.extend_from_slice(&4u16.to_le_bytes());
        b.extend_from_slice(&32u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&8u32.to_le_bytes());
        b.extend_from_slice(&0.5f32.to_le_bytes());
        b.extend_from_slice(&(-1.0f32).to_le_bytes());
        let pcm = decode(&b).unwrap();
        assert_eq!(pcm.rate, 24_000);
        assert_eq!(pcm.samples, [16383, -32767]);
    }

    #[test]
    fn voiced_seconds_ignore_silence() {
        let v = voiced_secs(&tone(24_000, 1.0, 2.0));
        assert!((v - 1.0).abs() < 0.05, "{v}");
    }

    #[test]
    fn join_adds_each_pause_and_resamples() {
        let a = tone(24_000, 0.5, 0.0);
        let b = tone(12_000, 0.5, 0.0);
        let joined = join(&[(a, 250), (b, 0)]);
        assert_eq!(joined.rate, 24_000);
        let secs = joined.samples.len() as f32 / 24_000.0;
        assert!((secs - 1.25).abs() < 0.01, "{secs}");
    }
}

/**
 * The clip Yinyue is spoken to with: what the microphone heard, made into the
 * one form `POST /api/stt` takes — WAV, 16 kHz, mono, 16-bit PCM, at most 60
 * seconds (doc design § 4). Pure, so node tests hold it; the control that
 * records (holdToTalk.mts) is its only caller.
 */

/** The one rate she hears at. */
export const SPEECH_RATE = 16_000;
/** The longest clip, seconds. */
export const MAX_SPEECH_SECONDS = 60;

/** Mono samples at `fromRate` as mono samples at 16 kHz, cut at 60 s. Going
 *  down, each output sample is the average of the input it covers (a plain
 *  low-pass, so voice above 8 kHz does not fold back into the band). */
export function toSpeechSamples(input: Float32Array, fromRate: number): Float32Array {
  const ratio = fromRate / SPEECH_RATE;
  const longest = SPEECH_RATE * MAX_SPEECH_SECONDS;
  const length = Math.min(Math.floor(input.length / ratio), longest);
  const out = new Float32Array(length);
  for (let i = 0; i < length; i++) {
    out[i] = ratio <= 1 ? interpolated(input, i * ratio) : averaged(input, i * ratio, (i + 1) * ratio);
  }
  return out;
}

function interpolated(input: Float32Array, at: number): number {
  const lo = Math.floor(at);
  const hi = Math.min(lo + 1, input.length - 1);
  return input[lo] + (input[hi] - input[lo]) * (at - lo);
}

/** The mean of `input` over [from, to), the two edges weighted by how much of
 *  their sample the span covers. */
function averaged(input: Float32Array, from: number, to: number): number {
  const first = Math.floor(from);
  const last = Math.min(Math.ceil(to), input.length);
  let sum = 0;
  let weight = 0;
  for (let k = first; k < last; k++) {
    const w = Math.min(k + 1, to) - Math.max(k, from);
    sum += input[k] * w;
    weight += w;
  }
  return weight > 0 ? sum / weight : 0;
}

/** 16 kHz mono samples (-1..1) as one WAV file: a 44-byte header, then
 *  16-bit little-endian PCM. */
export function encodeWav(samples: Float32Array): Uint8Array {
  const bytes = samples.length * 2;
  const wav = new Uint8Array(44 + bytes);
  const view = new DataView(wav.buffer);
  const tag = (at: number, text: string) => {
    for (let i = 0; i < text.length; i++) view.setUint8(at + i, text.charCodeAt(i));
  };
  tag(0, 'RIFF');
  view.setUint32(4, 36 + bytes, true);
  tag(8, 'WAVE');
  tag(12, 'fmt ');
  view.setUint32(16, 16, true); // fmt chunk size
  view.setUint16(20, 1, true); // PCM
  view.setUint16(22, 1, true); // mono
  view.setUint32(24, SPEECH_RATE, true);
  view.setUint32(28, SPEECH_RATE * 2, true); // bytes per second
  view.setUint16(32, 2, true); // bytes per frame
  view.setUint16(34, 16, true); // bits per sample
  tag(36, 'data');
  view.setUint32(40, bytes, true);
  for (let i = 0; i < samples.length; i++) {
    const s = Math.max(-1, Math.min(1, samples[i]));
    view.setInt16(44 + i * 2, Math.round(s < 0 ? s * 0x8000 : s * 0x7fff), true);
  }
  return wav;
}

/** Base64 of bytes, in slices (a spread of a clip-sized array overflows the stack). */
export function toBase64(bytes: Uint8Array): string {
  const SLICE = 0x8000;
  let binary = '';
  for (let i = 0; i < bytes.length; i += SLICE) {
    binary += String.fromCharCode(...bytes.subarray(i, i + SLICE));
  }
  return btoa(binary);
}

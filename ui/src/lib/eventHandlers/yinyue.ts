/**
 * Yinyue "speak" cue handler — the web surface's consumer of the one event
 * spine (see `doc/yinyue-spec.md` › Adaptive presentation).
 *
 * The engine pushes a small `{ text, emotion }` cue (`ServerEvent::YinyueSpeak`)
 * over the control channel; here we fetch the audio from `/api/tts` and play it,
 * show the speech bubble, and publish a live **mouth-opening** signal so the VRM
 * avatar can lip-sync in step with the audio it doesn't own.
 */
import type { UiEvent } from '../../types';
import { _originalFetch } from '../fetchProxy';
import { useUiStore } from '../../stores/uiStore';

// One shared AudioContext, created lazily on first cue.
let audioCtx: AudioContext | null = null;
let current: AudioBufferSourceNode | null = null;

// Lip-sync resolution: one RMS sample per ~frame (≈60fps).
const ENVELOPE_FRAME_MS = 16;
// The playing clip's loudness curve + the time it started, so any surface can
// read the current mouth opening by indexing the envelope at elapsed time.
let playback: { envelope: Float32Array; startTime: number } | null = null;

/** Pre-compute a per-frame RMS loudness envelope from a decoded clip. */
function computeRmsEnvelope(buf: AudioBuffer): Float32Array {
  const data = buf.getChannelData(0); // mono
  const frameLen = Math.max(1, Math.floor((buf.sampleRate * ENVELOPE_FRAME_MS) / 1000));
  const frames = Math.ceil(data.length / frameLen);
  const env = new Float32Array(frames);
  for (let f = 0; f < frames; f++) {
    const start = f * frameLen;
    const end = Math.min(start + frameLen, data.length);
    let sum = 0;
    for (let i = start; i < end; i++) sum += data[i] * data[i];
    env[f] = Math.sqrt(sum / Math.max(1, end - start));
  }
  return env;
}

/** Current mouth opening (0..1) for the clip playing right now, else 0.
 *  Read each frame by the VRM avatar to drive lip-sync. */
export function getMouthOpening(): number {
  if (!playback || !audioCtx) return 0;
  const frame = Math.floor((audioCtx.currentTime - playback.startTime) / (ENVELOPE_FRAME_MS / 1000));
  const amp = playback.envelope[Math.max(0, Math.min(frame, playback.envelope.length - 1))] ?? 0;
  // RMS of speech sits ~0..0.3; scale into a mouth opening.
  return Math.max(0, Math.min(0.06 + amp * 3.2, 1));
}

/** Pet expression cue — emotion and/or a one-shot gesture for the avatar. */
export function handlePetExpress(item: UiEvent): void {
  const emotion = item.data?.emotion as string | undefined;
  const action = item.data?.action as string | undefined;
  if (!emotion && !action) return;
  console.info(`[pet] express emotion=${emotion ?? '-'} action=${action ?? '-'}`);
  useUiStore.getState().pushPetExpress(emotion, action);
}

export function handlePetSpeak(item: UiEvent): void {
  const text = ((item.data?.text as string | undefined) ?? item.text ?? '').trim();
  if (!text) return;
  const emotion = (item.data?.emotion as string | undefined) ?? 'neutral';
  console.info(`[yinyue] speak (${emotion}): ${text}`);
  useUiStore.getState().showYinyueSpeech(text, emotion); // visual bubble + avatar emotion
  void play(text); // voice + mouth-signal
}

async function play(text: string): Promise<void> {
  try {
    // Direct HTTP (not the proxied window.fetch, which mangles binary over WebRTC).
    const resp = await _originalFetch('/api/tts', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ text }),
    });
    if (!resp.ok) throw new Error(`tts ${resp.status}`);
    const bytes = await resp.arrayBuffer();

    const ctx = (audioCtx ??= new AudioContext());
    if (ctx.state === 'suspended') await ctx.resume();
    const decoded = await ctx.decodeAudioData(bytes);

    current?.stop(); // cut off any clip still playing
    const src = ctx.createBufferSource();
    src.buffer = decoded;
    src.connect(ctx.destination);

    playback = { envelope: computeRmsEnvelope(decoded), startTime: ctx.currentTime };
    current = src;
    src.onended = () => {
      if (current === src) { current = null; playback = null; }
    };
    src.start();
  } catch (err) {
    console.error('[yinyue] speak failed', err);
  }
}

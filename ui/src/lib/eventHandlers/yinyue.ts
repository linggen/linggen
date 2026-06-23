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

// A gesture held until the voice for the same turn begins, so the action lands
// together with the bubble + voice (synthesis lags the events by a few seconds).
// While it's held she stays in idle / thinking. A silent gesture (no speech)
// fires on its own after a short fallback wait.
let pendingExpress: { emotion?: string; action?: string } | null = null;
let pendingTimer: number | undefined;
let synthInFlight = false;

function flushExpress(): void {
  if (pendingTimer !== undefined) {
    clearTimeout(pendingTimer);
    pendingTimer = undefined;
  }
  if (!pendingExpress) return;
  const { emotion, action } = pendingExpress;
  pendingExpress = null;
  useUiStore.getState().pushPetExpress(emotion, action);
}

/** Pet expression cue — emotion and/or a one-shot gesture for the avatar. Held
 *  until her voice begins so it lands with the bubble + voice; if no speech
 *  follows shortly it fires on its own (a silent gesture). */
export function handlePetExpress(item: UiEvent): void {
  const emotion = item.data?.emotion as string | undefined;
  const action = item.data?.action as string | undefined;
  if (!emotion && !action) return;
  console.info(`[pet] express emotion=${emotion ?? '-'} action=${action ?? '-'}`);

  // Already speaking → the gesture rides the current speech immediately.
  if (current) {
    useUiStore.getState().pushPetExpress(emotion, action);
    return;
  }
  // Otherwise hold it (she stays in idle/thinking) until the voice starts.
  pendingExpress = { emotion, action };
  if (pendingTimer !== undefined) clearTimeout(pendingTimer);
  pendingTimer = undefined;
  // No speech is being synthesized → treat as a silent gesture; fire it soon.
  // (If speech starts first, play() cancels this and waits for the voice onset.)
  if (!synthInFlight) {
    pendingTimer = window.setTimeout(() => {
      useUiStore.getState().setPetThinking(false);
      flushExpress();
    }, 500);
  }
}

export function handlePetSpeak(item: UiEvent): void {
  const text = ((item.data?.text as string | undefined) ?? item.text ?? '').trim();
  if (!text) return;
  const emotion = (item.data?.emotion as string | undefined) ?? 'neutral';
  console.info(`[yinyue] speak (${emotion}): ${text}`);
  // Keep her in thinking until the voice actually starts (synth lags a few
  // seconds); the bubble, voice, and any held gesture all land together then.
  void play(text, emotion);
}

async function play(text: string, emotion: string): Promise<void> {
  // Speech is coming → hold any pending gesture for the voice onset (not the
  // short silent-gesture fallback), and keep her in thinking until then.
  synthInFlight = true;
  if (pendingTimer !== undefined) {
    clearTimeout(pendingTimer);
    pendingTimer = undefined;
  }

  // The moment her audio is ready: leave thinking, show the bubble, and fire any
  // held gesture — so action + bubble + voice all land together.
  const onAudioStart = () => {
    synthInFlight = false;
    useUiStore.getState().setPetThinking(false);
    useUiStore.getState().setPetSpeaking(true); // talking body loop while the voice plays
    useUiStore.getState().showYinyueSpeech(text, emotion);
    flushExpress();
  };

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
      if (current === src) {
        current = null;
        playback = null;
        useUiStore.getState().setPetSpeaking(false); // voice done → drop the talking loop
      }
    };
    onAudioStart(); // bubble + gesture land exactly as the voice begins
    src.start();
  } catch (err) {
    // TTS failed — show text, leave thinking, fire the gesture; no audio so no talking loop.
    console.error('[yinyue] speak failed', err);
    synthInFlight = false;
    useUiStore.getState().setPetThinking(false);
    useUiStore.getState().setPetSpeaking(false);
    useUiStore.getState().showYinyueSpeech(text, emotion);
    flushExpress();
  }
}

/**
 * Yinyue "speak" cue handler — the web surface's consumer of the one event
 * spine (see `doc/yinyue-spec.md` › Adaptive presentation).
 *
 * The engine pushes a small `{ text, emotion }` cue (`ServerEvent::YinyueSpeak`)
 * over the control channel, with the line already cut into short clauses. Here
 * we fetch each clause's audio from `/api/tts` — the first plays as soon as it
 * is made while the next is synthesized — show the speech bubble, and publish a
 * live **mouth-opening** signal so the VRM avatar can lip-sync in step with the
 * audio it doesn't own.
 */
import type { UiEventOf } from '../../types';
import type { SpokenChunk } from '../../types/generated/SpokenChunk';
import { useUiStore } from '../../stores/uiStore';

// One shared AudioContext, created lazily on first cue.
let audioCtx: AudioContext | null = null;
// The clauses of the current line scheduled on the context and not yet ended.
let queued: AudioBufferSourceNode[] = [];

// Lip-sync resolution: one RMS sample per ~frame (≈60fps).
const ENVELOPE_FRAME_MS = 16;
// Each scheduled clause's loudness curve + its start time, so any surface can
// read the current mouth opening by indexing the envelope at elapsed time.
let segments: { envelope: Float32Array; startTime: number }[] = [];

// Bubble floor per spoken character while the rest of the line is still being
// made. Her lines measured 280–340 ms per character with pauses (2026-10-22);
// this errs long so the bubble never leaves before the voice.
const BUBBLE_MS_PER_CHAR = 380;

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
  if (!audioCtx) return 0;
  const now = audioCtx.currentTime;
  const seg = segments.findLast((s) => s.startTime <= now);
  if (!seg) return 0;
  const frame = Math.floor((now - seg.startTime) / (ENVELOPE_FRAME_MS / 1000));
  // Past the clause's end = the pause before the next one: mouth closed.
  if (frame >= seg.envelope.length) return 0;
  const amp = seg.envelope[Math.max(0, frame)] ?? 0;
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
/** Per-peer presenter push: does THIS surface hold the FCFS singleton lock?
 *  Drives whether her avatar/bubble mount (parent gates on the store flag) and
 *  guards her voice/expression below. */
export function handleYinyuePresent(item: UiEventOf<'yinyue_present'>): void {
  const present = !!(item.data as { present?: boolean } | undefined)?.present;
  // Her voice follows the lock: a surface that hands her over goes quiet at
  // once — the clip it is playing and any line still being synthesized. Two
  // Lingjing tabs spoke at once (2026-09-24): each stage reload moves the lock,
  // and the old holder kept talking over the new one.
  if (!present) hush();
  useUiStore.getState().setYinyuePresenter(present);
}

/// Every clip this surface started or is still synthesizing; bumped to drop
/// the in-flight ones.
let voiceGeneration = 0;

/// Stop her voice here: the playing clip, and any line still on its way.
function hush(): void {
  voiceGeneration++;
  synthInFlight = false;
  stopQueued();
  useUiStore.getState().setPetSpeaking(false);
}

/// Silence every clause already scheduled.
function stopQueued(): void {
  const playing = queued;
  queued = [];
  segments = [];
  for (const src of playing) src.stop();
}

/// Whether a clip started at `generation` may still sound on this surface.
function mayStillSpeak(generation: number): boolean {
  return generation === voiceGeneration && useUiStore.getState().yinyuePresenter;
}

export function handlePetExpress(item: UiEventOf<'pet_express'>): void {
  // Singleton guard: only the presenter renders her (server already unicasts
  // pet events to the holder; this is belt-and-suspenders across handoffs).
  if (!useUiStore.getState().yinyuePresenter) return;
  const emotion = item.data?.emotion as string | undefined;
  const action = item.data?.action as string | undefined;
  if (!emotion && !action) return;
  console.info(`[pet] express emotion=${emotion ?? '-'} action=${action ?? '-'}`);

  // Already speaking → the gesture rides the current speech immediately.
  if (queued.length > 0) {
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

export function handlePetSpeak(item: UiEventOf<'pet_speak'>): void {
  // Singleton guard: only the presenter speaks (see handlePetExpress).
  if (!useUiStore.getState().yinyuePresenter) return;
  const text = ((item.data?.text as string | undefined) ?? item.text ?? '').trim();
  if (!text) return;
  const emotion = (item.data?.emotion as string | undefined) ?? 'neutral';
  console.info(`[yinyue] speak (${emotion}): ${text}`);
  // Her voice is muted on this Mac: the words, no audio fetched.
  if (item.data?.voice === false) {
    showWords(text, emotion);
    return;
  }
  // Keep her in thinking until the voice actually starts (synth lags a few
  // seconds); the bubble, voice, and any held gesture all land together then.
  const chunks = (item.data?.chunks as SpokenChunk[] | undefined) ?? [];
  void play(text, emotion, chunks.length > 0 ? chunks : [{ text, pause_ms: 0 }]);
}

/// Her voice was turned off or on (`/mute`, her Voice tool, Settings). Every
/// surface hears it: muting cuts off whatever she is saying now.
export function handlePetVoice(item: UiEventOf<'pet_voice'>): void {
  if (item.data?.muted !== true) return;
  hush();
}

/// Her line as words only — the bubble and any held gesture, no talking loop.
function showWords(text: string, emotion: string): void {
  synthInFlight = false;
  useUiStore.getState().setPetThinking(false);
  useUiStore.getState().setPetSpeaking(false);
  useUiStore.getState().showYinyueSpeech(text, emotion);
  flushExpress();
}

/// One clause's audio, over the data channel (the WAV comes back as bytes).
async function fetchClause(text: string): Promise<ArrayBuffer> {
  const resp = await fetch('/api/tts', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ text }),
  });
  if (!resp.ok) throw new Error(`tts ${resp.status}`);
  return resp.arrayBuffer();
}

/// How long the bubble must stay at least, before the line's real length is
/// known: its characters at a slow reading pace, plus every pause.
function estimateLineMs(chunks: SpokenChunk[]): number {
  return chunks.reduce((ms, c) => ms + c.text.length * BUBBLE_MS_PER_CHAR + c.pause_ms, 0);
}

async function play(text: string, emotion: string, chunks: SpokenChunk[]): Promise<void> {
  // Speech is coming → hold any pending gesture for the voice onset (not the
  // short silent-gesture fallback), and keep her in thinking until then.
  synthInFlight = true;
  // A new line supersedes any line still being made: its remaining clauses
  // stop scheduling, and what is sounding stops when this line's audio starts.
  const generation = ++voiceGeneration;
  if (pendingTimer !== undefined) {
    clearTimeout(pendingTimer);
    pendingTimer = undefined;
  }

  // The moment her audio is ready: leave thinking, show the bubble, and fire any
  // held gesture — so action + bubble + voice all land together.
  const onAudioStart = (audioMs: number) => {
    synthInFlight = false;
    useUiStore.getState().setPetThinking(false);
    useUiStore.getState().setPetSpeaking(true); // talking body loop while the voice plays
    // The bubble must outlive the voice: floor its dismiss countdown at the
    // line's (estimated) duration plus a beat — the reading-time formula alone
    // under-shoots spoken Chinese.
    useUiStore.getState().showYinyueSpeech(text, emotion, audioMs + 2500);
    flushExpress();
  };

  let started = false;
  let lineDone = false;
  // The talking loop ends with the line's last clause, not with a gap between
  // clauses while the next one is still being made.
  const settle = (src?: AudioBufferSourceNode) => {
    if (src) queued = queued.filter((q) => q !== src);
    if (lineDone && queued.length === 0 && generation === voiceGeneration) {
      segments = [];
      useUiStore.getState().setPetSpeaking(false);
    }
  };

  try {
    const ctx = (audioCtx ??= new AudioContext());
    let next = fetchClause(chunks[0].text);
    let queueEnd = 0;
    for (let i = 0; i < chunks.length; i++) {
      const bytes = await next;
      // The lock moved, she was muted, or a newer line began while this clause
      // was made: the newer owner speaks; this line stays quiet.
      if (!mayStillSpeak(generation)) return;
      // Pipeline: the next clause is made while this one decodes and plays.
      if (i + 1 < chunks.length) {
        next = fetchClause(chunks[i + 1].text);
        next.catch(() => undefined); // awaited below; never an unhandled rejection
      }
      if (ctx.state === 'suspended') await ctx.resume();
      const decoded = await ctx.decodeAudioData(bytes);
      if (!mayStillSpeak(generation)) return;

      if (!started) {
        stopQueued(); // cut off the previous line, if still sounding
        queueEnd = ctx.currentTime;
      }
      const at = Math.max(ctx.currentTime, queueEnd);
      const src = ctx.createBufferSource();
      src.buffer = decoded;
      src.connect(ctx.destination);
      src.onended = () => settle(src);
      src.start(at);
      queued.push(src);
      segments.push({ envelope: computeRmsEnvelope(decoded), startTime: at });
      queueEnd = at + decoded.duration + chunks[i].pause_ms / 1000;

      if (!started) {
        started = true;
        onAudioStart(Math.max(decoded.duration * 1000, estimateLineMs(chunks)));
      }
    }
  } catch (err) {
    console.error('[yinyue] speak failed', err);
    // Nothing sounded yet → her words, no audio, no talking loop. Mid-line →
    // the bubble is up; she simply stops after what was said.
    if (!started && mayStillSpeak(generation)) showWords(text, emotion);
  } finally {
    lineDone = true;
    settle();
  }
}

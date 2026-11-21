/**
 * Hold to talk: the one control that turns a press on Yinyue into a line said
 * to her (design § 3). Press and hold → "Listening…" → let go → "Thinking…" →
 * her answer. This module owns press → record → hear → send → the words shown;
 * the page avatar and the pet window both bind it (components/yinyue/
 * YinyueAvatar.tsx, through lib/yinyueTalk.ts) and neither records.
 *
 * The microphone is opened here and nowhere else (a test fails on a second
 * `getUserMedia` in src/), and only while held: release, cancel, the window
 * hidden or blurred, the page unloaded, the 60 s cap — each stops the tracks.
 * What is sent to hear is one WAV clip (lib/speechWav.mts) to `POST /api/stt`
 * on this machine's own engine; what was heard goes to her door as a typed
 * line would, marked `asked_aloud`.
 *
 * Pure of the page: everything it touches (microphone, audio context, fetch,
 * timers, the window) is handed in, so node tests drive it with fakes.
 */
import { BUDGET_HEADER } from './requestBudget.mts';
import { encodeWav, MAX_SPEECH_SECONDS, SPEECH_RATE, toBase64, toSpeechSamples } from './speechWav.mts';

/** A press shorter than this is a tap — the avatar's own click, unchanged. */
export const HOLD_MS = 250;
/** Moving the pointer this far before the hold begins is a drag, not a talk. */
export const DRAG_PX = 4;
/** A clip shorter than this holds no sentence; nothing is sent. */
const MIN_CLIP_MS = 250;
/** How long a quiet line ("Didn't catch that.") stays. */
export const NOTE_MS = 4000;
/** How long the heard line stays after it was sent to her. */
export const HEARD_MS = 20_000;
/** After a 503, a press shows its reason without opening the microphone. */
export const UNAVAILABLE_MS = 15_000;
/** One hearing may take this long (the model's turn comes later, on her door). */
const HEAR_BUDGET_MS = 60_000;

/** What the person reads. Each is the whole line, at most 20 words. */
export const WORDS = {
  listening: 'Listening…',
  thinking: 'Thinking…',
  nothing: "Didn't catch that.",
  cannotHear: "She can't hear on this Mac right now.",
  micBlocked: 'The microphone is blocked. Allow it for Linggen, then try again.',
  noMic: "This page can't use the microphone.",
  failed: "Couldn't reach her. Try again.",
} as const;

export type TalkPhase =
  | 'idle'
  | 'pressed' // down, not yet a hold
  | 'opening' // a hold; the microphone is being asked for
  | 'listening' // the microphone is open
  | 'refused' // a hold that cannot record (the machine cannot hear, no microphone); waits for the release
  | 'hearing' // let go; the clip is being heard, then sent
  | 'replying'; // sent; her answer is on its way

/** What the surface draws: the phase, the words heard, one quiet line. */
export interface TalkView {
  phase: TalkPhase;
  heard: string | null;
  note: string | null;
}

/** "Listening…" / "Thinking…" / nothing — told once each (the third state,
 *  speaking, is her voice and bubble). `petThinking` is the store's flag that a
 *  reply is in flight, so "Thinking…" ends when her line arrives. */
export function talkLabel(view: TalkView, petThinking: boolean): string | null {
  switch (view.phase) {
    case 'listening': return WORDS.listening;
    case 'hearing': return WORDS.thinking;
    case 'replying': return petThinking ? WORDS.thinking : null;
    default: return null;
  }
}

/** A key that starts a talk: the space bar alone, never inside a field or a
 *  button (where it types or presses), never as key repeat. */
export function isTalkKey(e: { code: string; repeat: boolean; metaKey: boolean; ctrlKey: boolean; altKey: boolean; target: unknown }): boolean {
  if (e.code !== 'Space' || e.repeat || e.metaKey || e.ctrlKey || e.altKey) return false;
  const t = e.target as { tagName?: string; isContentEditable?: boolean } | null;
  if (t?.isContentEditable) return false;
  return !['INPUT', 'TEXTAREA', 'SELECT', 'BUTTON', 'A'].includes(t?.tagName?.toUpperCase() ?? '');
}

// ── What is handed in ──────────────────────────────────────────────────

interface TrackLike { stop(): void }
interface StreamLike { getTracks(): TrackLike[] }
interface NodeLike { connect(to: unknown): unknown; disconnect(): void }
interface ProcessorLike extends NodeLike {
  onaudioprocess: ((e: { inputBuffer: { getChannelData(channel: number): Float32Array } }) => void) | null;
}
export interface AudioContextLike {
  sampleRate: number;
  state: string;
  destination: unknown;
  resume(): Promise<void>;
  close(): Promise<void>;
  createMediaStreamSource(stream: StreamLike): NodeLike;
  createScriptProcessor(bufferSize: number, inputs: number, outputs: number): ProcessorLike;
}
interface Listenable {
  addEventListener(type: string, fn: () => void): void;
  removeEventListener(type: string, fn: () => void): void;
}

export interface TalkDeps {
  /** `navigator.mediaDevices`, or undefined where the page has none. */
  mediaDevices: () => { getUserMedia(c: unknown): Promise<StreamLike> } | undefined;
  newAudioContext: () => AudioContextLike;
  /** The page's own `fetch` (data channel); read at call time. */
  fetch: (url: string, init: { method: string; headers: Record<string, string>; body: string }) => Promise<{ ok: boolean; status: number; json(): Promise<unknown> }>;
  /** Her door: the line as a typed one would go, marked asked aloud. True when it went. */
  sendLine: (text: string) => Promise<boolean>;
  /** The one stop her mute uses. */
  cutOff: () => void;
  now: () => number;
  setTimer: (fn: () => void, ms: number) => unknown;
  clearTimer: (id: unknown) => void;
  win: Listenable;
  doc: Listenable & { hidden: boolean };
}

type Source = 'pointer' | 'key';

/** The microphone, open: stops its tracks, hands back what it heard. */
interface Take { stop(): { samples: Float32Array; rate: number } }

export interface HoldToTalk {
  view(): TalkView;
  subscribe(fn: () => void): () => void;
  /** Pointer down or key down. */
  press(source: Source): void;
  /** Pointer up or key up: 'tap' if it never became a hold (let the avatar's
   *  click proceed), 'talk' if it did (swallow that click). */
  release(source: Source): 'tap' | 'talk';
  /** Drop a press that has not been heard yet: microphone off, nothing sent. */
  cancel(): void;
}

export function createHoldToTalk(deps: TalkDeps): HoldToTalk {
  // A press and what follows it are two things: a new press can begin while the
  // last clip is still being heard, and a tap leaves that hearing alone.
  let held: 'none' | 'pressed' | 'opening' | 'listening' | 'refused' = 'none';
  let after: 'none' | 'hearing' | 'replying' = 'none';
  let heard: string | null = null;
  let note: string | null = null;
  let source: Source | null = null;
  let generation = 0; // bumped when a hold starts or ends: older answers are dropped
  let take: Take | null = null;
  let unavailable: { reason: string; until: number } | null = null;
  const phaseNow = (): TalkPhase => (held !== 'none' ? held : after === 'none' ? 'idle' : after);
  let snapshot: TalkView = { phase: 'idle', heard, note };
  const listeners = new Set<() => void>();
  const timers: { hold?: unknown; cap?: unknown; note?: unknown; heard?: unknown } = {};

  const emit = () => {
    snapshot = { phase: phaseNow(), heard, note };
    for (const fn of [...listeners]) fn();
  };
  const stopTimer = (key: keyof typeof timers) => {
    if (timers[key] !== undefined) deps.clearTimer(timers[key]);
    timers[key] = undefined;
  };
  const startTimer = (key: keyof typeof timers, ms: number, fn: () => void) => {
    stopTimer(key);
    timers[key] = deps.setTimer(() => { timers[key] = undefined; fn(); }, ms);
  };

  // While a press is live the window leaving ends it — blur, hidden, unloaded.
  const leave = () => cancel();
  const leaveIfHidden = () => { if (deps.doc.hidden) cancel(); };
  let watching = false;
  const watch = (on: boolean) => {
    if (on === watching) return;
    watching = on;
    const [win, doc] = [deps.win, deps.doc];
    if (on) {
      win.addEventListener('blur', leave);
      win.addEventListener('pagehide', leave);
      doc.addEventListener('visibilitychange', leaveIfHidden);
    } else {
      win.removeEventListener('blur', leave);
      win.removeEventListener('pagehide', leave);
      doc.removeEventListener('visibilitychange', leaveIfHidden);
    }
  };

  /** The press is over (whatever way): no key or pointer holds it, the window is not watched. */
  const unpress = () => { source = null; held = 'none'; stopTimer('hold'); stopTimer('cap'); watch(false); };

  const say = (line: string) => {
    note = line;
    startTimer('note', NOTE_MS, () => { note = null; emit(); });
  };
  /** One quiet line, and nothing half-open: the phase is idle. */
  const fail = (line: string) => {
    heard = null;
    after = 'none';
    say(line);
    emit();
  };

  async function openTake(): Promise<Take> {
    const devices = deps.mediaDevices();
    if (!devices?.getUserMedia) throw new Error('no microphone api');
    const stream = await devices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true } });
    const stopTracks = () => { for (const t of stream.getTracks()) t.stop(); };
    try {
      const ctx = deps.newAudioContext();
      if (ctx.state === 'suspended') void ctx.resume().catch(() => undefined);
      const source = ctx.createMediaStreamSource(stream);
      // A ScriptProcessor: deprecated, but it runs in every webview we ship in
      // without serving a worklet file, and the clip is one minute at most.
      const processor = ctx.createScriptProcessor(4096, 1, 1);
      const chunks: Float32Array[] = [];
      processor.onaudioprocess = (e) => { chunks.push(new Float32Array(e.inputBuffer.getChannelData(0))); };
      source.connect(processor);
      processor.connect(ctx.destination);
      return {
        stop() {
          processor.onaudioprocess = null;
          source.disconnect();
          processor.disconnect();
          stopTracks();
          void ctx.close().catch(() => undefined);
          const samples = new Float32Array(chunks.reduce((n, c) => n + c.length, 0));
          let at = 0;
          for (const c of chunks) { samples.set(c, at); at += c.length; }
          return { samples, rate: ctx.sampleRate };
        },
      };
    } catch (e) {
      stopTracks();
      throw e;
    }
  }

  async function beginHold(): Promise<void> {
    timers.hold = undefined;
    const mine = ++generation; // an earlier hearing's answer is dropped
    stopTimer('note');
    stopTimer('heard');
    heard = null;
    note = null;
    after = 'none';
    deps.cutOff(); // pressing to talk stops what she is saying
    if (unavailable && deps.now() < unavailable.until) {
      held = 'refused';
      say(unavailable.reason);
      emit();
      return;
    }
    held = 'opening';
    emit();
    let opened: Take;
    try {
      opened = await openTake();
    } catch (e) {
      if (mine !== generation) return;
      held = 'refused'; // still held; the release ends it
      say(isBlocked(e) ? WORDS.micBlocked : WORDS.noMic);
      emit();
      return;
    }
    if (mine !== generation) { opened.stop(); return; } // let go (or hidden) while it was opening
    take = opened;
    held = 'listening';
    startTimer('cap', MAX_SPEECH_SECONDS * 1000, () => finishHold());
    emit();
  }

  function finishHold(): void {
    const t = take;
    take = null;
    unpress();
    if (!t) return;
    const mine = ++generation;
    const { samples, rate } = t.stop();
    const clip = toSpeechSamples(samples, rate);
    if (clip.length < (SPEECH_RATE * MIN_CLIP_MS) / 1000) {
      fail(WORDS.nothing);
      return;
    }
    after = 'hearing';
    emit();
    void hear(mine, encodeWav(clip));
  }

  async function hear(mine: number, wav: Uint8Array): Promise<void> {
    let reply;
    try {
      // The data channel carries JSON, so the clip rides as base64 (`wav_b64`).
      reply = await deps.fetch('/api/stt', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', [BUDGET_HEADER]: String(HEAR_BUDGET_MS) },
        body: JSON.stringify({ wav_b64: toBase64(wav) }),
      });
    } catch {
      if (mine === generation) fail(WORDS.failed);
      return;
    }
    if (mine !== generation) return;
    const body = (await reply.json().catch(() => ({}))) as { text?: unknown; reason?: unknown };
    if (mine !== generation) return;
    if (reply.status === 503) {
      const reason = typeof body.reason === 'string' && body.reason ? body.reason : WORDS.cannotHear;
      unavailable = { reason, until: deps.now() + UNAVAILABLE_MS };
      fail(reason);
      return;
    }
    if (!reply.ok) return fail(WORDS.failed);
    const text = typeof body.text === 'string' ? body.text.trim() : '';
    if (!text) return fail(WORDS.nothing);

    heard = text; // shown at once, so a mis-hearing is seen, not guessed
    emit();
    const sent = await deps.sendLine(text);
    if (mine !== generation) return;
    if (!sent) return fail(WORDS.failed);
    after = 'replying';
    startTimer('heard', HEARD_MS, () => { heard = null; after = 'none'; emit(); });
    emit();
  }

  function press(by: Source): void {
    if (source !== null) return; // one press at a time
    source = by;
    held = 'pressed';
    watch(true);
    startTimer('hold', HOLD_MS, () => { void beginHold(); });
    emit();
  }

  function release(by: Source): 'tap' | 'talk' {
    if (source !== by) return 'tap';
    if (held === 'pressed') {
      unpress();
      emit();
      return 'tap';
    }
    if (held === 'listening') {
      finishHold();
      return 'talk';
    }
    // Refused, or the microphone still opening: nothing is heard.
    generation++;
    unpress();
    emit();
    return 'talk';
  }

  function cancel(): void {
    if (source === null) return;
    if (held !== 'pressed') generation++; // a press that never became a hold leaves a hearing alone
    const t = take;
    take = null;
    t?.stop();
    unpress();
    emit();
  }

  return {
    view: () => snapshot,
    subscribe(fn) {
      listeners.add(fn);
      return () => { listeners.delete(fn); };
    },
    press,
    release,
    cancel,
  };
}

function isBlocked(e: unknown): boolean {
  const name = (e as { name?: string } | null)?.name;
  return name === 'NotAllowedError' || name === 'SecurityError';
}

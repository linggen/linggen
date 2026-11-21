/**
 * Hold to talk: the one control that turns a press on Yinyue into a line said
 * to her (design § 3). Press and hold → "Listening…" → let go → "Thinking…" →
 * her answer. This module owns press → record → hear → send → the words shown;
 * the page avatar and the pet window both bind it (components/yinyue/
 * YinyueAvatar.tsx, through lib/yinyueTalk.ts) and neither records.
 *
 * Whether she can hear is asked of the engine (`GET /api/stt`) at the hold,
 * before the microphone opens: on a Mac that cannot, the press shows the reason
 * and records nothing. The microphone is opened here and nowhere else (a test
 * fails on a second `getUserMedia` in src/), and only while held: release, cancel, the window
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

/** A press shorter than this is a tap — the avatar's own click, unchanged.
 *  The one place this is set: 350 ms so a slow click is still a click; tune it
 *  here after the first try on a real Mac (e2e holds wait on it by name). */
export const HOLD_MS = 350;
/** Moving the pointer this far before the hold begins is a drag, not a talk. */
export const DRAG_PX = 4;
/** A clip shorter than this holds no sentence; nothing is sent. */
const MIN_CLIP_MS = 250;
/** How long a quiet line ("Didn't catch that.") stays. */
export const NOTE_MS = 4000;
/** How long the heard line stays after it was sent to her. */
export const HEARD_MS = 20_000;
/** How long a "yes, she can hear" is kept before the next press asks again.
 *  A "no" is never kept: the model may have arrived since. A 503 from a clip
 *  forgets the yes at once (the state can change between the two requests). */
export const HEARS_MS = 60_000;
/** One hearing may take this long (the model's turn comes later, on her door):
 *  the engine's wait for the slot and for the model, at most a minute's clip
 *  included, fits inside it (`stt_mlx`'s lock reads this number). */
export const HEAR_BUDGET_MS = 90_000;

/** What the person reads. Each is the whole line, at most 20 words. */
export const WORDS = {
  listening: 'Listening…',
  thinking: 'Thinking…',
  nothing: "Didn't catch that.",
  cannotHear: "She can't hear on this Mac right now.",
  micBlocked: 'The microphone is blocked. Allow it for Linggen, then try again.',
  micAllowed: 'Microphone allowed. Hold again to talk.',
  noMic: "This page can't use the microphone.",
  failed: "Couldn't reach her. Try again.",
} as const;

export type TalkPhase =
  | 'idle'
  | 'pressed' // down, not yet a hold
  | 'opening' // a hold; the microphone is being asked for
  | 'listening' // the microphone is open
  | 'refused' // a hold that cannot record (she cannot hear on this Mac, no microphone); waits for the release
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
/** The recorder inside the audio context: it posts the microphone's samples in
 *  blocks (lib/speechTap.ts), off the page's render thread. */
interface TapLike extends NodeLike {
  port: { onmessage: ((e: { data: Float32Array }) => void) | null };
}
export interface AudioContextLike {
  sampleRate: number;
  destination: unknown;
  resume(): Promise<void>;
  suspend(): Promise<void>;
  /** The recorder is loaded into the context (once). */
  ready(): Promise<void>;
  createMediaStreamSource(stream: StreamLike): NodeLike;
  createTap(): TapLike;
}
interface Listenable {
  addEventListener(type: string, fn: () => void): void;
  removeEventListener(type: string, fn: () => void): void;
}

/** What her door did with a line: took it, or refused — with its own one line
 *  of reason when it gave one (a spoken line cannot go to an app's chat). */
export type LineSent = { sent: true } | { sent: false; reason: string | null };

export interface TalkDeps {
  /** `navigator.mediaDevices`, or undefined where the page has none. */
  mediaDevices: () => { getUserMedia(c: unknown): Promise<StreamLike> } | undefined;
  /** Made inside the press, where the page allows a sound context to run. */
  newAudioContext: () => AudioContextLike;
  /** Whether holding her can ever be granted a microphone here (a stage frame
   *  in the app cannot); when not, a press stays a tap. */
  offered: () => boolean;
  /** The page's own `fetch` (data channel); read at call time. */
  fetch: (url: string, init: { method: string; headers: Record<string, string>; body?: string }) => Promise<{ ok: boolean; status: number; json(): Promise<unknown> }>;
  /** Her door: the line as a typed one would go, marked asked aloud. */
  sendLine: (text: string) => Promise<LineSent>;
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
  /** Whether a press here can become a talk at all. */
  offered(): boolean;
}

export function createHoldToTalk(deps: TalkDeps): HoldToTalk {
  // A press and what follows it are two things: a new press can begin while the
  // last clip is still being heard, and a tap leaves that hearing alone.
  let held: 'none' | 'pressed' | 'opening' | 'listening' | 'refused' = 'none';
  let after: 'none' | 'hearing' | 'replying' = 'none';
  let heard: string | null = null;
  let note: string | null = null;
  let source: Source | null = null;
  // The 60 s cap sent the clip while the press was still down: its release is
  // still a talk's (the avatar swallows the click), and does nothing more.
  let lingering: Source | null = null;
  let generation = 0; // bumped when a hold starts or ends: older answers are dropped
  let take: Take | null = null;
  // The generation the window's leaving ended while the microphone was being
  // asked for: the system's permission dialog takes focus from the page.
  let windowTook: number | null = null;
  let toldAllowed = false; // "Microphone allowed. Hold again" is said once
  // The page's one audio context, made inside a press (the page lets sound run
  // only from a gesture) and idled between talks.
  let audio: { ctx: AudioContextLike; ready: Promise<void> } | null = null;
  let hearsUntil = 0; // she could hear when last asked, until this time
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
  const leave = () => cancel('window');
  const leaveIfHidden = () => { if (deps.doc.hidden) cancel('window'); };
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

  /** Wake the audio context — inside the press, the one moment the page lets
   *  it run. Made once, resumed at every press. */
  function wakeAudio(): void {
    try {
      if (!audio) {
        const ctx = deps.newAudioContext();
        const ready = ctx.ready();
        ready.catch(() => { audio = null; }); // the next press tries again
        audio = { ctx, ready };
      }
      void audio.ctx.resume().catch(() => undefined);
    } catch {
      audio = null;
    }
  }
  const idleAudio = () => { void audio?.ctx.suspend().catch(() => undefined); };

  async function openTake(): Promise<Take> {
    const devices = deps.mediaDevices();
    if (!devices?.getUserMedia) throw new Error('no microphone api');
    const wake = audio;
    if (!wake) throw new Error('no audio context');
    await wake.ready;
    const stream = await devices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true } });
    const stopTracks = () => { for (const t of stream.getTracks()) t.stop(); };
    try {
      const { ctx } = wake;
      const source = ctx.createMediaStreamSource(stream);
      // An AudioWorklet taps the samples off the page's render thread, which
      // the avatar's renderer shares: a stall there would chop the speech.
      const tap = ctx.createTap();
      const chunks: Float32Array[] = [];
      tap.port.onmessage = (e) => { chunks.push(e.data); };
      source.connect(tap);
      tap.connect(ctx.destination);
      return {
        stop() {
          tap.port.onmessage = null;
          source.disconnect();
          tap.disconnect();
          stopTracks();
          idleAudio();
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

  /** One request to `/api/stt`; null when the channel failed. */
  async function request(method: 'GET' | 'POST', init: { headers: Record<string, string>; body?: string }) {
    try {
      const reply = await deps.fetch('/api/stt', { method, ...init });
      const body = (await reply.json().catch(() => ({}))) as Record<string, unknown>;
      return { ok: reply.ok, status: reply.status, body };
    } catch {
      return null;
    }
  }

  const reasonOf = (body: Record<string, unknown>) => (typeof body.reason === 'string' && body.reason ? body.reason : null);

  /** Can she hear on this Mac: asked once at a hold, kept a minute when yes.
   *  A no that is not a fetch on its way sends one knock — a request with no
   *  clip — because the first real press is what starts the model's fetch (the
   *  question itself starts nothing); what the knock answers is the reason. */
  async function canHear(): Promise<{ hears: true } | { hears: false; reason: string }> {
    if (deps.now() < hearsUntil) return { hears: true };
    const asked = await request('GET', { headers: {} });
    if (!asked?.ok) return { hears: false, reason: WORDS.failed };
    if (asked.body.hears === true) {
      hearsUntil = deps.now() + HEARS_MS;
      return { hears: true };
    }
    const reason = reasonOf(asked.body) ?? WORDS.cannotHear;
    if (asked.body.fetching === true) return { hears: false, reason };
    const knocked = await request('POST', { headers: { 'Content-Type': 'application/json' }, body: '{}' });
    return { hears: false, reason: (knocked?.status === 503 ? reasonOf(knocked.body) : null) ?? reason };
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
    held = 'opening';
    emit();
    const verdict = await canHear();
    if (mine !== generation) return; // let go (or hidden) while she was asked
    if (!verdict.hears) {
      held = 'refused'; // still held; the release ends it
      say(verdict.reason);
      emit();
      return;
    }
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
    if (mine !== generation) {
      opened.stop(); // let go (or hidden) while it was opening
      // The permission dialog took the page's focus and ended this press: the
      // first allow is a press the person must make again — told once.
      if (windowTook === mine && !toldAllowed) {
        toldAllowed = true;
        say(WORDS.micAllowed);
        emit();
      }
      return;
    }
    take = opened;
    held = 'listening';
    startTimer('cap', MAX_SPEECH_SECONDS * 1000, () => {
      const by = source;
      finishHold();
      lingering = by;
    });
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
      hearsUntil = 0;
      fail(reasonOf(body) ?? WORDS.cannotHear);
      return;
    }
    if (!reply.ok) return fail(WORDS.failed);
    const text = typeof body.text === 'string' ? body.text.trim() : '';
    if (!text) return fail(WORDS.nothing);

    heard = text; // shown at once, so a mis-hearing is seen, not guessed
    emit();
    const sent = await deps.sendLine(text);
    if (mine !== generation) return;
    if (!sent.sent) return fail(sent.reason ?? WORDS.failed);
    after = 'replying';
    startTimer('heard', HEARD_MS, () => { heard = null; after = 'none'; emit(); });
    emit();
  }

  function press(by: Source): void {
    if (source !== null || !deps.offered()) return; // one press at a time
    lingering = null;
    source = by;
    held = 'pressed';
    wakeAudio();
    watch(true);
    startTimer('hold', HOLD_MS, () => { void beginHold(); });
    emit();
  }

  function release(by: Source): 'tap' | 'talk' {
    if (lingering === by) {
      lingering = null;
      return 'talk';
    }
    if (source !== by) return 'tap';
    if (held === 'pressed') {
      unpress();
      idleAudio();
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
    idleAudio();
    emit();
    return 'talk';
  }

  function cancel(by: 'window' | 'other' = 'other'): void {
    if (source === null) return;
    if (by === 'window' && held === 'opening') windowTook = generation;
    if (held !== 'pressed') generation++; // a press that never became a hold leaves a hearing alone
    const t = take;
    take = null;
    t?.stop();
    unpress();
    idleAudio();
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
    cancel: () => cancel(),
    offered: deps.offered,
  };
}

function isBlocked(e: unknown): boolean {
  const name = (e as { name?: string } | null)?.name;
  return name === 'NotAllowedError' || name === 'SecurityError';
}

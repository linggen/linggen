// hold-to-talk.js — hold to talk: the one control that turns a press into a
// line said aloud and heard. Press and hold → "Listening…" → let go →
// "Thinking…" → the words heard go to the page's `sendLine`. Served by the
// engine at /shared/hold-to-talk.js for any app page that holds the
// microphone, and bundled by the main UI through the `@linggen/hold-to-talk`
// alias (ui/vite.config.ts) for Yinyue's avatar: one copy, no second.
//
//   import { createHoldToTalk, newSpeechContext } from '/shared/hold-to-talk.js';
//   const talk = createHoldToTalk({
//     mediaDevices: () => navigator.mediaDevices,
//     newAudioContext: newSpeechContext,
//     offered: () => true,
//     fetch: (url, init) => window.fetch(url, init),
//     sendLine: async (text) => { chat.send(text, { heard: true }); return { sent: true }; },
//     cutOff: () => {},
//     now: () => Date.now(),
//     setTimer: (fn, ms) => window.setTimeout(fn, ms),
//     clearTimer: (id) => window.clearTimeout(id),
//     win: window,
//     doc: document,
//   });
//
// Whether the engine can hear is asked of it (`GET /api/stt`) at the hold,
// before the microphone opens: on a Mac that cannot, the press shows the
// reason and records nothing. The microphone is opened here and nowhere else
// (a test fails on a second `getUserMedia` in this folder or in ui/src), and
// only while held: release, cancel, the window hidden or blurred, the page
// unloaded, the 60 s cap — each stops the tracks. What is sent to hear is one
// WAV clip (16 kHz mono, below) to `POST /api/stt` on this machine's own
// engine; what was heard goes to `sendLine` as a typed line would.
//
// Pure of the page: everything it touches (microphone, audio context, fetch,
// timers, the window) is handed in, so node tests drive it with fakes
// (tests/shared/hold-to-talk.test.mjs). One file by design — the control, the
// WAV encoder and the recorder are never loaded apart.

/** The fetch header a call names its wait in, in milliseconds. The same
 *  literal is the channel's (shared/channel.js) and the UI's policy
 *  (ui/src/lib/requestBudget.mts); a test holds the three equal
 *  (tests/shared/hold-to-talk.test.mjs). */
const BUDGET_HEADER = 'X-Linggen-Budget-Ms';

// ── The clip ──

// ── The clip: what the microphone heard, in the one form `POST /api/stt`
// takes — WAV, 16 kHz, mono, 16-bit PCM, at most 60 seconds ──

/** The one rate she hears at. */
export const SPEECH_RATE = 16_000;
/** The longest clip, seconds. */
export const MAX_SPEECH_SECONDS = 60;

/** Mono samples at `fromRate` as mono samples at 16 kHz, cut at 60 s. Going
 *  down, each output sample is the average of the input it covers (a plain
 *  low-pass, so voice above 8 kHz does not fold back into the band). */
export function toSpeechSamples(input, fromRate) {
  const ratio = fromRate / SPEECH_RATE;
  const longest = SPEECH_RATE * MAX_SPEECH_SECONDS;
  const length = Math.min(Math.floor(input.length / ratio), longest);
  const out = new Float32Array(length);
  for (let i = 0; i < length; i++) {
    out[i] = ratio <= 1 ? interpolated(input, i * ratio) : averaged(input, i * ratio, (i + 1) * ratio);
  }
  return out;
}

function interpolated(input, at) {
  const lo = Math.floor(at);
  const hi = Math.min(lo + 1, input.length - 1);
  return input[lo] + (input[hi] - input[lo]) * (at - lo);
}

/** The mean of `input` over [from, to), the two edges weighted by how much of
 *  their sample the span covers. */
function averaged(input, from, to) {
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
export function encodeWav(samples) {
  const bytes = samples.length * 2;
  const wav = new Uint8Array(44 + bytes);
  const view = new DataView(wav.buffer);
  const tag = (at, text) => {
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
export function toBase64(bytes) {
  const SLICE = 0x8000;
  let binary = '';
  for (let i = 0; i < bytes.length; i += SLICE) {
    binary += String.fromCharCode(...bytes.subarray(i, i + SLICE));
  }
  return btoa(binary);
}

// ── The control: press → record → hear → send → the words shown ──

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
};

/**
 * @typedef {'idle'
 *   | 'pressed'    down, not yet a hold
 *   | 'opening'    a hold; the microphone is being asked for
 *   | 'listening'  the microphone is open
 *   | 'refused'    a hold that cannot record (she cannot hear on this Mac, no microphone); waits for the release
 *   | 'hearing'    let go; the clip is being heard, then sent
 *   | 'replying'   sent; her answer is on its way} TalkPhase
 */

/**
 * What the surface draws: the phase, the words heard, one quiet line.
 * @typedef {{ phase: TalkPhase, heard: string | null, note: string | null }} TalkView
 */

/**
 * What the engine says of her hearing, asked of it (`GET /api/stt`) and never guessed.
 * `next` is the engine's own word for what would help: `'never'` (this Mac cannot hear),
 * `'download'` (files are missing: a `start()` downloads them — `reason` names the size),
 * `'working'` (a fetch or load is under way), `'downloading'` (a language's model is being put on
 * this Mac: a wait like `'working'`), `'retry'` (nothing missing: a `start()` only
 * loads, or tries again); `null` when she can hear. `reason` is the engine's sentence, as it
 * said it. When the engine could not be reached: `hears` false, `next` `'retry'`, `reason` WORDS.failed, `failed` true.
 * @typedef {{ hears: boolean, next: 'never'|'download'|'working'|'downloading'|'retry'|null, reason: string|null, failed?: true }} HearingStatus
 */

/** "Listening…" / "Thinking…" / nothing — told once each (the third state,
 *  speaking, is her voice and bubble). `petThinking` is the store's flag that a
 *  reply is in flight, so "Thinking…" ends when her line arrives. */
export function talkLabel(view, petThinking) {
  switch (view.phase) {
    case 'listening': return WORDS.listening;
    case 'hearing': return WORDS.thinking;
    case 'replying': return petThinking ? WORDS.thinking : null;
    default: return null;
  }
}

/** A key that starts a talk: the space bar alone, never inside a field or a
 *  button (where it types or presses), never as key repeat. */
export function isTalkKey(e) {
  if (e.code !== 'Space' || e.repeat || e.metaKey || e.ctrlKey || e.altKey) return false;
  const t = e.target;
  if (t?.isContentEditable) return false;
  return !['INPUT', 'TEXTAREA', 'SELECT', 'BUTTON', 'A'].includes(t?.tagName?.toUpperCase() ?? '');
}

// ── What is handed in ──────────────────────────────────────────────────
//
// `deps` (the shapes are in ui/src/types/hold-to-talk.d.ts, which the UI's
// compiler reads; this file is plain JavaScript):
//   mediaDevices()      `navigator.mediaDevices`, or undefined where the page has none.
//   newAudioContext()   made inside the press, where the page allows a sound context
//                       to run; `newSpeechContext` below is the page's real one. Its
//                       recorder (`createTap`) posts the microphone's samples in
//                       blocks, off the page's render thread.
//   offered()           whether holding can ever be granted a microphone here (a stage
//                       frame in the app cannot); when not, a press stays a tap.
//   fetch(url, init)    the page's own `fetch` (data channel); read at call time.
//   sendLine(text)      the line as a typed one would go. Resolves `{ sent: true }`,
//                       or `{ sent: false, reason }` — its own one line of reason when
//                       it refused (a spoken line cannot go to an app's chat).
//   cutOff()            the one stop the page's mute uses: pressing stops what is said.
//   now, setTimer, clearTimer, win, doc   the clock and the window, handed in.
//   language            optional: the language the clip is in, a code (`en`, `zh-Hans`)
//                       or `'auto'` (tell the hearing model nothing). Without it the
//                       engine decides (Settings, else the last line typed to her) — right
//                       for Yinyue, wrong for a learner, who speaks the language being learned.
//   knock               optional, default true: whether a hold that finds her unable to hear
//                       sends the empty request that starts the model's fetch. An app page that
//                       must not download without a yes passes false, asks `status()` itself, and
//                       calls `start()` only from that yes.
// A "take" below is the microphone, open: `stop()` ends its tracks and hands
// back what it heard.

/**
 * One hold to talk, bound to a page by what it is handed.
 * @param {object} deps  see above
 * @returns {{
 *   view(): TalkView,
 *   subscribe(fn: () => void): () => void,
 *   press(source: 'pointer' | 'key'): void,
 *   release(source: 'pointer' | 'key'): 'tap' | 'talk',
 *   cancel(): void,
 *   offered(): boolean,
 *   status(): Promise<HearingStatus>,
 *   start(): Promise<HearingStatus>,
 * }}  `status`: ask whether she can hear — read only, starts nothing. `start`: begin what
 *   hearing waits on (the model's fetch, or its load) — the one call that may download, made
 *   only from a person's yes; answers what `status` then says. `press`: pointer down or key down. `release`: pointer up or key up — 'tap' if
 *   it never became a hold (let the avatar's click proceed), 'talk' if it did (swallow
 *   that click). `cancel`: drop a press that has not been heard yet — microphone off,
 *   nothing sent. `offered`: whether a press here can become a talk at all.
 */
export function createHoldToTalk(deps) {
  // A press and what follows it are two things: a new press can begin while the
  // last clip is still being heard, and a tap leaves that hearing alone.
  let held = 'none';
  let after = 'none';
  let heard = null;
  let note = null;
  let source = null;
  // The 60 s cap sent the clip while the press was still down: its release is
  // still a talk's (the avatar swallows the click), and does nothing more.
  let lingering = null;
  let generation = 0; // bumped when a hold starts or ends: older answers are dropped
  let take = null;
  // The generation the window's leaving ended while the microphone was being
  // asked for: the system's permission dialog takes focus from the page.
  let windowTook = null;
  let toldAllowed = false; // "Microphone allowed. Hold again" is said once
  // The page's one audio context, made inside a press (the page lets sound run
  // only from a gesture) and idled between talks.
  let audio = null;
  let hearsUntil = 0; // she could hear when last asked, until this time
  const phaseNow = () => (held !== 'none' ? held : after === 'none' ? 'idle' : after);
  let snapshot = { phase: 'idle', heard, note };
  const listeners = new Set ();
  const timers = {};

  const emit = () => {
    snapshot = { phase: phaseNow(), heard, note };
    for (const fn of [...listeners]) fn();
  };
  const stopTimer = (key) => {
    if (timers[key] !== undefined) deps.clearTimer(timers[key]);
    timers[key] = undefined;
  };
  const startTimer = (key, ms, fn) => {
    stopTimer(key);
    timers[key] = deps.setTimer(() => { timers[key] = undefined; fn(); }, ms);
  };

  // While a press is live the window leaving ends it — blur, hidden, unloaded.
  const leave = () => cancel('window');
  const leaveIfHidden = () => { if (deps.doc.hidden) cancel('window'); };
  let watching = false;
  const watch = (on) => {
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

  const say = (line) => {
    note = line;
    startTimer('note', NOTE_MS, () => { note = null; emit(); });
  };
  /** One quiet line, and nothing half-open: the phase is idle. */
  const fail = (line) => {
    heard = null;
    after = 'none';
    say(line);
    emit();
  };

  /** Wake the audio context — inside the press, the one moment the page lets
   *  it run. Made once, resumed at every press. */
  function wakeAudio() {
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

  async function openTake() {
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
      const chunks = [];
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
  async function request(method, init) {
    try {
      const reply = await deps.fetch('/api/stt', { method, ...init });
      const body = await reply.json().catch(() => ({}));
      return { ok: reply.ok, status: reply.status, body };
    } catch {
      return null;
    }
  }

  const reasonOf = (body) => (typeof body.reason === 'string' && body.reason ? body.reason : null);

  /** What the engine's answer to a "no" says, for a page. */
  const noOf = (body) => ({
    hears: false,
    next: ['never', 'download', 'working', 'downloading', 'retry'].includes(body.next) ? body.next : body.fetching === true ? 'working' : 'retry',
    reason: reasonOf(body) ?? WORDS.cannotHear,
  });

  /** Ask the engine whether she can hear (read only). */
  async function ask() {
    const asked = await request('GET', { headers: {} });
    if (!asked?.ok) return { hears: false, next: 'retry', reason: WORDS.failed, failed: true };
    if (asked.body.hears === true) return { hears: true, next: null, reason: null };
    return noOf(asked.body);
  }

  /** The knock: a request with no clip. Its one use is to begin the fetch or the load hearing waits on
   *  (`POST /api/stt` begins it before it looks at the body); what it answers is the reason. */
  const knock = () => request('POST', { headers: { 'Content-Type': 'application/json' }, body: '{}' });

  /** Can she hear on this Mac: asked once at a hold, kept a minute when yes.
   *  A no that is not a fetch on its way sends one knock (unless the page opted out with
   *  `knock: false`) because the first real press is what starts the model's fetch (the
   *  question itself starts nothing); what the knock answers is the reason. */
  async function canHear() {
    if (deps.now() < hearsUntil) return { hears: true };
    const status = await ask();
    if (status.hears) {
      hearsUntil = deps.now() + HEARS_MS;
      return { hears: true };
    }
    if (status.failed || status.next === 'working' || status.next === 'downloading' || deps.knock === false) return { hears: false, reason: status.reason };
    const knocked = await knock();
    return { hears: false, reason: (knocked?.status === 503 ? reasonOf(knocked.body) : null) ?? status.reason };
  }

  /** A person's yes: begin what hearing waits on, then say how she stands. */
  async function start() {
    await knock();
    return ask();
  }

  async function beginHold() {
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
    let opened;
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

  function finishHold() {
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

  async function hear(mine, wav) {
    let reply;
    try {
      // The data channel carries JSON, so the clip rides as base64 (`wav_b64`).
      reply = await deps.fetch('/api/stt', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', [BUDGET_HEADER]: String(HEAR_BUDGET_MS) },
        body: JSON.stringify(deps.language ? { wav_b64: toBase64(wav), language: deps.language } : { wav_b64: toBase64(wav) }),
      });
    } catch {
      if (mine === generation) fail(WORDS.failed);
      return;
    }
    if (mine !== generation) return;
    const body = await reply.json().catch(() => ({}));
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

  function press(by) {
    if (source !== null || !deps.offered()) return; // one press at a time
    lingering = null;
    source = by;
    held = 'pressed';
    wakeAudio();
    watch(true);
    startTimer('hold', HOLD_MS, () => { void beginHold(); });
    emit();
  }

  function release(by) {
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

  function cancel(by = 'other') {
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
    status: ask,
    start,
  };
}

function isBlocked(e) {
  const name = (e)?.name;
  return name === 'NotAllowedError' || name === 'SecurityError';
}

// ── The microphone's recorder ──
//
// The page's audio context for hearing: an AudioWorklet taps the microphone's
// samples in blocks and posts them to the page, so capture never runs on the
// thread that draws her. The worklet's code is a string made into a Blob URL —
// no second file to serve, and nothing fetched over the network.
//
// Why a worklet and not MediaRecorder: the recorder yields a compressed
// container (webm or mp4 by webview) that must be decoded again before it can
// be a 16 kHz WAV, which is another async step that can fail per webview; the
// worklet hands over the raw samples the WAV needs, and stopping it is
// synchronous, as the microphone-off paths require.

/** Samples per posted block: ~21 ms at 48 kHz, the most a release can clip. */
const BLOCK = 1024;

const WORKLET = `
class SpeechTap extends AudioWorkletProcessor {
  constructor() { super(); this.block = new Float32Array(${BLOCK}); this.at = 0; }
  process(inputs) {
    const channel = inputs[0] && inputs[0][0];
    if (channel) {
      for (let i = 0; i < channel.length; i++) {
        this.block[this.at++] = channel[i];
        if (this.at === ${BLOCK}) {
          this.port.postMessage(this.block, [this.block.buffer]);
          this.block = new Float32Array(${BLOCK});
          this.at = 0;
        }
      }
    }
    return true;
  }
}
registerProcessor('speech-tap', SpeechTap);
`;

/** A fresh audio context with the recorder ready to load. */
export function newSpeechContext() {
  const Ctor = window.AudioContext ?? window.webkitAudioContext;
  const ctx = new Ctor();
  return {
    get sampleRate() { return ctx.sampleRate; },
    destination: ctx.destination,
    resume: () => ctx.resume(),
    suspend: () => ctx.suspend(),
    async ready() {
      const url = URL.createObjectURL(new Blob([WORKLET], { type: 'text/javascript' }));
      try {
        await ctx.audioWorklet.addModule(url);
      } finally {
        URL.revokeObjectURL(url);
      }
    },
    createMediaStreamSource: (stream) => ctx.createMediaStreamSource(stream),
    createTap: () => new AudioWorkletNode(ctx, 'speech-tap', { numberOfInputs: 1, numberOfOutputs: 1, channelCount: 1 }),
  };
}

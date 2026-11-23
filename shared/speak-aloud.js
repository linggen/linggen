// speak-aloud.js — read a line aloud in the engine's own voice. Text in, her voice out
// (`POST /api/tts`, one WAV per clause), with `stop()` so a new press of the microphone
// cuts her off. Served by the engine at /shared/speak-aloud.js for any app page.
//
//   import { createSpeakAloud, newPlayerContext } from '/shared/speak-aloud.js';
//   const voice = createSpeakAloud({
//     fetch: (url, init) => window.fetch(url, init),
//     newAudioContext: newPlayerContext,
//   });
//   const now = await voice.status();   // { speaks, natural, reason } — read only
//   voice.prime();                      // inside a click or a press: the page lets sound run
//   voice.say('Nice to meet you.');     // returns when she has finished, or was cut off
//   voice.stop();                       // press to talk, a switch turned off, a page leaving
//
// Whether this Mac can speak is asked of the engine (`GET /api/tts`) and starts nothing: no
// fetch, no model load, no clip made. `natural` says the line will be in her own voice;
// without it the engine speaks with a system voice. The page decides WHEN a line is read
// (the lesson's rule is scripts/voice.mjs); this file only reads what it is handed.
//
// Two players read a line out of /api/tts and play it: this one, for an app's page, and
// the companion's (ui/src/lib/eventHandlers/yinyue.ts `play`), which is tied to her avatar
// and the pet store (lip-sync, bubble, gestures). Two users, so both stay (doc/code-style.md
// principle 5); a change in how a line is cut into clauses or queued is looked at in both.
//
// Pure of the page: the fetch and the audio context are handed in, so node tests drive it
// with fakes (tests/shared/speak-aloud.test.mjs).

/** The longest clause asked of the voice, in characters. The engine cuts a line again
 *  (it drifts on long ones); this only keeps the first sound near. */
export const CLAUSE_CHARS = 240;

/** What a reply is as it is said: Markdown marks, code and links reduced to the words. */
export function plainSpeech(text) {
  return String(text ?? '')
    .replace(/```[\s\S]*?```/g, ' ')
    .replace(/`([^`]*)`/g, '$1')
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/^\s{0,3}(#{1,6}|>|[-*+]|\d+[.)])\s+/gm, '')
    .replace(/[*_~]{1,3}/g, '')
    .replace(/[ \t]+/g, ' ')
    .trim();
}

// A full stop inside a number (3.5) or a name (U.S.A) is not an end: ASCII marks end a sentence before a space.
const SENTENCE_END = /(?<=[.!?…])\s+|(?<=[。！？])\s*|\n+/;

/** A line cut into the clauses she is asked for one by one: at a sentence end or a line
 *  break, short ones joined up to [CLAUSE_CHARS], a long one cut at a space or a comma. */
export function clausesOf(text) {
  const pieces = plainSpeech(text).split(SENTENCE_END).map((p) => p.trim()).filter(Boolean);
  const clauses = [];
  for (const piece of pieces.flatMap(cutLong)) {
    const last = clauses.length - 1;
    if (last >= 0 && clauses[last].length + 1 + piece.length <= CLAUSE_CHARS) clauses[last] += (/[。！？]$/.test(clauses[last]) ? '' : ' ') + piece;
    else clauses.push(piece);
  }
  return clauses;
}

function cutLong(piece) {
  const out = [];
  let rest = piece;
  while (rest.length > CLAUSE_CHARS) {
    const window = rest.slice(0, CLAUSE_CHARS);
    const at = Math.max(window.lastIndexOf(' '), window.lastIndexOf(','), window.lastIndexOf('，'), window.lastIndexOf('、'));
    const cut = at > CLAUSE_CHARS / 2 ? at + 1 : CLAUSE_CHARS;
    out.push(rest.slice(0, cut).trim());
    rest = rest.slice(cut).trim();
  }
  if (rest) out.push(rest);
  return out;
}

/**
 * One reader. `deps`:
 *   fetch(url, init)    the page's own `fetch` (data channel); read at call time.
 *   newAudioContext()   the page's audio context (`newPlayerContext`); made on first use.
 * @returns {{
 *   status(): Promise<{ speaks: boolean, natural: boolean, reason: string|null, failed?: true }>,
 *   prime(): void,
 *   say(text: string): Promise<{ spoken: boolean }>,
 *   stop(): void,
 *   speaking(): boolean,
 *   subscribe(fn: () => void): () => void,
 * }}
 */
export function createSpeakAloud(deps) {
  let generation = 0; // bumped by every say and stop: an older line's work is dropped
  let queued = [];
  let endLine = null; // ends the line being said (its `say` returns), when stop or a newer line comes
  let ctx = null;
  let speaking = false;
  const listeners = new Set();
  const emit = () => { for (const fn of [...listeners]) fn(); };
  const setSpeaking = (on) => { if (speaking !== on) { speaking = on; emit(); } };

  async function status() {
    try {
      const reply = await deps.fetch('/api/tts', { method: 'GET', headers: {} });
      const body = await reply.json().catch(() => ({}));
      if (!reply.ok) return { speaks: false, natural: false, reason: null, failed: true };
      return {
        speaks: body.speaks === true,
        natural: body.natural === true,
        reason: typeof body.reason === 'string' && body.reason ? body.reason : null,
      };
    } catch {
      return { speaks: false, natural: false, reason: null, failed: true };
    }
  }

  function context() {
    ctx ??= deps.newAudioContext();
    return ctx;
  }

  /** Inside a click or a press, the one moment a page lets sound run. */
  function prime() {
    try { void context().resume().catch(() => undefined); } catch { /* the page has no sound */ }
  }

  async function clip(text) {
    const reply = await deps.fetch('/api/tts', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ text }),
    });
    if (!reply.ok) throw new Error(`tts ${reply.status}`);
    return reply.arrayBuffer();
  }

  function stopSounds() {
    const sounding = queued;
    queued = [];
    for (const src of sounding) {
      src.onended = null;
      try { src.stop(); } catch { /* it had ended */ }
    }
  }

  function endCurrent() {
    const end = endLine;
    endLine = null;
    end?.();
  }

  async function say(text) {
    const mine = ++generation;
    stopSounds();
    endCurrent();
    const clauses = clausesOf(text);
    if (!clauses.length) { setSpeaking(false); return { spoken: false }; }
    setSpeaking(true);
    let finished = null;
    const ended = new Promise((resolve) => { finished = resolve; });
    endLine = finished;
    let playing = 0;
    let allQueued = false;
    const settle = () => { if (allQueued && playing === 0) finished(); };
    let any = false;
    try {
      const audio = context();
      if (audio.state === 'suspended') await audio.resume();
      let next = clip(clauses[0]);
      let queueEnd = 0;
      for (let i = 0; i < clauses.length; i++) {
        const bytes = await next;
        if (mine !== generation) return { spoken: false };
        // Pipeline: the next clause is made while this one decodes and plays.
        if (i + 1 < clauses.length) {
          next = clip(clauses[i + 1]);
          next.catch(() => undefined); // awaited above; never an unhandled rejection
        }
        const buffer = await audio.decodeAudioData(bytes);
        if (mine !== generation) return { spoken: false };
        const at = Math.max(audio.currentTime, queueEnd);
        const src = audio.createBufferSource();
        src.buffer = buffer;
        src.connect(audio.destination);
        playing++;
        src.onended = () => { playing--; queued = queued.filter((q) => q !== src); settle(); };
        src.start(at);
        queued.push(src);
        queueEnd = at + buffer.duration;
        any = true;
      }
      allQueued = true;
      settle();
      await ended;
      return { spoken: mine === generation && any };
    } catch {
      // Nothing is said, or she stops where she was: never a sentence of the page's own.
      return { spoken: any && mine === generation };
    } finally {
      if (mine === generation) setSpeaking(false);
    }
  }

  function stop() {
    generation++;
    stopSounds();
    endCurrent();
    setSpeaking(false);
  }

  return {
    status,
    prime,
    say,
    stop,
    speaking: () => speaking,
    subscribe(fn) {
      listeners.add(fn);
      return () => { listeners.delete(fn); };
    },
  };
}

/** The page's audio context for playing a line. */
export function newPlayerContext() {
  const Ctor = window.AudioContext ?? window.webkitAudioContext;
  return new Ctor();
}

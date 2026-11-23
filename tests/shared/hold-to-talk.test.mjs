// /shared/hold-to-talk.js — the control the main UI and any app page share:
// the microphone is open only while held, what is sent is one 16 kHz mono WAV
// to /api/stt, and each state is told once. Everything the control touches is
// a fake: no microphone, no sound. Run: node --test tests/shared/*.test.mjs
// (the page wiring of Yinyue's avatar is ui/tests/holdToTalkWiring.test.mts).
import assert from 'node:assert/strict';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  createHoldToTalk, HEARD_MS, HOLD_MS, HEARS_MS, isTalkKey, NOTE_MS, talkLabel, WORDS,
} from '../../shared/hold-to-talk.js';

const flush = () => new Promise((r) => setImmediate(r));

/** A manual clock: timers run only when the test says time has passed. */
function clock() {
  let now = 1_000_000;
  let next = 1;
  const timers = new Map();
  return {
    now: () => now,
    setTimer: (fn, ms) => { timers.set(next, { at: now + ms, fn }); return next++; },
    clearTimer: (id) => { timers.delete(id); },
    async advance(ms) {
      const end = now + ms;
      for (;;) {
        const due = [...timers.entries()].filter(([, t]) => t.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
        if (!due) break;
        timers.delete(due[0]);
        now = due[1].at;
        due[1].fn();
        await flush();
      }
      now = end;
      await flush();
    },
  };
}

/**
 * The control with every dependency faked. What the rig holds, for the tests:
 *   talk, clk, win, doc    the control, a manual clock, the window and document
 *   fetched, sent, views   every fetch (url, init), every line sent on, every view told
 *   cuts                   how often the page's one stop was asked
 *   streams, asked, open() microphone requests, and the tracks not yet stopped
 *   contexts, gestureWhenMade, inPress, audio   audio contexts made (and whether
 *                          inside a press), and resume / suspend in order
 *   workletFails, offered  switches for the recorder and for holding being offered
 *   heardAs, hearsAs       what POST / GET /api/stt answer ('throw', or 'wait' for a GET
 *                          held until answerHears(a))
 *   sendOk, sendReason     what the page's door said to the line (and why it refused)
 *   gate()                 holds the microphone request until release() / fail(name)
 *   speak(ms, rate)        the recorder posts a block of sound
 */
function rig() {
  const clk = clock();
  const win = new EventTarget();
  const doc = Object.assign(new EventTarget(), { hidden: false });
  const r = {
    fetched: [], sent: [], cuts: 0, streams: [], asked: 0,
    heardAs: { value: { status: 200, body: { text: 'what is on today' } } },
    contexts: 0, gestureWhenMade: [], inPress: false, audio: [], workletFails: false, offered: true,
    sendOk: { value: true }, sendReason: { value: null }, views: [],
    hearsAs: { value: { status: 200, body: { hears: true } } },
  };
  let heldGet = null;
  let gated = null;
  let processor = null;
  let rate = 48_000;
  const deps = {
    mediaDevices: () => ({
      async getUserMedia() {
        r.asked++;
        if (gated) await gated.promise;
        const stream = { tracks: [{ stopped: 0 }] };
        r.streams.push(stream);
        return { getTracks: () => stream.tracks.map((t) => ({ stop: () => { t.stopped++; } })) };
      },
    }),
    newAudioContext: () => {
      r.contexts++;
      r.gestureWhenMade.push(r.inPress);
      return {
        get sampleRate() { return rate; },
        destination: {},
        resume: async () => { r.audio.push('resume'); },
        suspend: async () => { r.audio.push('suspend'); },
        ready: async () => { if (r.workletFails) throw new Error('no worklet'); },
        createMediaStreamSource: () => ({ connect() { return undefined; }, disconnect() { /* off */ } }),
        createTap: () => {
          const p = { port: { onmessage: null }, connect() { return undefined; }, disconnect() { /* off */ } };
          processor = p;
          return p;
        },
      };
    },
    offered: () => r.offered,
    fetch: async (url, init) => {
      r.fetched.push({ url, init });
      const asking = init.method === 'GET';
      const answer = asking ? r.hearsAs.value : r.heardAs.value;
      if (answer === 'throw') throw new Error('channel down');
      if (answer === 'wait') {
        const a = await new Promise((res) => { heldGet = res; });
        return { ok: a.status >= 200 && a.status < 300, status: a.status, json: async () => a.body };
      }
      const { status, body } = answer;
      return { ok: status >= 200 && status < 300, status, json: async () => body };
    },
    sendLine: async (text) => { r.sent.push(text); return r.sendOk.value ? { sent: true } : { sent: false, reason: r.sendReason.value }; },
    cutOff: () => { r.cuts++; },
    now: clk.now, setTimer: clk.setTimer, clearTimer: clk.clearTimer, win, doc,
  };
  const answerHears = (a) => { heldGet?.(a); heldGet = null; };
  const talk = createHoldToTalk(deps);
  talk.subscribe(() => r.views.push(talk.view()));
  return Object.assign(r, {
    talk, clk, win, doc, answerHears,
    open: () => r.streams.flatMap((s) => s.tracks).filter((t) => t.stopped === 0).length,
    gate() {
      let resolve;
      let reject;
      const promise = new Promise((res, rej) => { resolve = res; reject = rej; });
      gated = { resolve, reject, promise };
      return {
        release: () => { gated = null; resolve(); },
        fail: (name) => { gated = null; reject(Object.assign(new Error(name), { name })); },
      };
    },
    speak(ms, sampleRate = 48_000) {
      rate = sampleRate;
      const frames = Math.round((ms / 1000) * sampleRate);
      const chunk = Float32Array.from({ length: frames }, (_, i) => 0.3 * Math.sin(i / 20));
      processor?.port.onmessage?.({ data: chunk });
    },
  });
}

/** Press, wait out the hold, and be listening. */
async function hold(r, by = 'pointer') {
  press(r, by);
  await r.clk.advance(HOLD_MS);
}

/** A press, the way the page gives it: the audio context is made only inside it. */
function press(r, by = 'pointer') {
  r.inPress = true;
  r.talk.press(by);
  r.inPress = false;
}

const phase = (r) => r.talk.view().phase;
/** The requests that carry a clip, or knock for one: not the question whether she hears. */
const posts = (r) => r.fetched.filter((f) => f.init.method === 'POST');
const asks = (r) => r.fetched.filter((f) => f.init.method === 'GET');

// ── Rule 6: a tap is a tap ─────────────────────────────────────────────

test('a press shorter than the hold is a tap: no microphone, no cut-off, no request', async () => {
  const r = rig();
  press(r);
  await r.clk.advance(HOLD_MS - 50);
  assert.equal(r.talk.release('pointer'), 'tap');
  await r.clk.advance(HOLD_MS * 4);
  assert.equal(r.asked, 0);
  assert.equal(r.cuts, 0);
  assert.equal(r.fetched.length, 0, 'a tap asks nothing of the engine either');
  assert.equal(phase(r), 'idle');
});

test('a hold is told so on release, so the avatar swallows its click', async () => {
  const r = rig();
  await hold(r);
  assert.equal(r.talk.release('pointer'), 'talk');
});

test('a release by the other kind of press is not this press\'s release', async () => {
  const r = rig();
  await hold(r, 'key');
  assert.equal(r.talk.release('pointer'), 'tap');
  assert.equal(phase(r), 'listening');
  r.talk.cancel();
});

// ── The states, told once each ─────────────────────────────────────────

test('Listening while held, Thinking from release, then her turn; the heard line shows', async () => {
  const r = rig();
  await hold(r);
  assert.equal(phase(r), 'listening');
  assert.equal(talkLabel(r.talk.view(), false), 'Listening…');
  r.speak(1200);
  r.talk.release('pointer');
  assert.equal(talkLabel(r.talk.view(), false), 'Thinking…');
  await r.clk.advance(0);
  assert.deepEqual(r.sent, ['what is on today']);
  assert.equal(r.talk.view().heard, 'what is on today');
  assert.equal(phase(r), 'replying');
  assert.equal(talkLabel(r.talk.view(), true), 'Thinking…', 'while her reply is in flight');
  assert.equal(talkLabel(r.talk.view(), false), null, 'her line arrived: no Thinking');
  await r.clk.advance(HEARD_MS);
  assert.equal(r.talk.view().heard, null);
});

test('pressing to talk cuts her off, once, when the hold begins', async () => {
  const r = rig();
  press(r);
  assert.equal(r.cuts, 0, 'a tap would not cut her off');
  await r.clk.advance(HOLD_MS);
  assert.equal(r.cuts, 1);
  r.talk.cancel();
});

// ── Rule 3: what is sent ───────────────────────────────────────────────

test('what is sent is one WAV, 16 kHz mono 16-bit, base64 in JSON, to /api/stt', async () => {
  const r = rig();
  await hold(r);
  r.speak(1500, 48_000);
  r.talk.release('pointer');
  await r.clk.advance(0);
  assert.equal(posts(r).length, 1);
  const { url, init } = posts(r)[0];
  assert.equal(url, '/api/stt');
  assert.equal(init.method, 'POST');
  assert.equal(init.headers['Content-Type'], 'application/json');
  const wav = Buffer.from(JSON.parse(init.body ?? '{}').wav_b64, 'base64');
  assert.equal(wav.toString('latin1', 0, 4), 'RIFF');
  assert.equal(wav.readUInt16LE(22), 1, 'mono');
  assert.equal(wav.readUInt32LE(24), 16_000);
  assert.equal(wav.readUInt16LE(34), 16);
  assert.equal(wav.readUInt32LE(40), 1500 * 16 * 2, 'a second and a half at 16 kHz, 2 bytes a sample');
});

test('nothing but the one /api/stt request carries sound, and it stays on this origin', async () => {
  const r = rig();
  await hold(r);
  r.speak(800);
  r.talk.release('pointer');
  await r.clk.advance(0);
  for (const f of r.fetched) assert.match(f.url, /^\/api\//, 'a path on the engine, never another host');
  assert.equal(r.fetched.filter((f) => f.init.body?.includes('wav_b64')).length, 1);
});

test('at 60 seconds the clip is stopped and sent without a release', async () => {
  const r = rig();
  await hold(r);
  r.speak(1000);
  await r.clk.advance(60_000);
  assert.equal(r.open(), 0);
  assert.equal(posts(r).length, 1);
  assert.equal(r.talk.release('pointer'), 'talk', 'the press was already ended, but it was a hold: its click is not a tap');
  assert.equal(r.talk.release('pointer'), 'tap');
  assert.equal(posts(r).length, 1, 'the release sent nothing more');
});

// ── Rule 2: the microphone is open only while held ─────────────────────

test('release stops the tracks', async () => {
  const r = rig();
  await hold(r);
  assert.equal(r.open(), 1);
  r.speak(500);
  r.talk.release('pointer');
  assert.equal(r.open(), 0);
});

test('cancel stops the tracks and sends nothing', async () => {
  const r = rig();
  await hold(r);
  r.speak(900);
  r.talk.cancel();
  await r.clk.advance(0);
  assert.equal(r.open(), 0);
  assert.equal(posts(r).length, 0);
  assert.equal(phase(r), 'idle');
});

for (const [name, end] of [
  ['the window loses focus', (r) => r.win.dispatchEvent(new Event('blur'))],
  ['the page is unloaded', (r) => r.win.dispatchEvent(new Event('pagehide'))],
  ['the window is hidden', (r) => { r.doc.hidden = true; r.doc.dispatchEvent(new Event('visibilitychange')); }],
]) {
  test(`${name}: the tracks stop, nothing is sent`, async () => {
    const r = rig();
    await hold(r);
    r.speak(900);
    end(r);
    await r.clk.advance(0);
    assert.equal(r.open(), 0);
    assert.equal(posts(r).length, 0);
    assert.equal(phase(r), 'idle');
  });
}

test('visibilitychange while still visible does nothing', async () => {
  const r = rig();
  await hold(r);
  r.doc.dispatchEvent(new Event('visibilitychange'));
  assert.equal(phase(r), 'listening');
  r.talk.cancel();
});

test('let go before the microphone answered: it is stopped the moment it does', async () => {
  const r = rig();
  const g = r.gate();
  await hold(r);
  assert.equal(phase(r), 'opening');
  assert.equal(r.talk.release('pointer'), 'talk');
  g.release();
  await r.clk.advance(0);
  assert.equal(r.streams.length, 1);
  assert.equal(r.open(), 0, 'a stream that arrives late is stopped');
  assert.equal(posts(r).length, 0);
  assert.equal(phase(r), 'idle');
});

test('cancelled or hidden before the microphone answered: stopped on arrival too', async () => {
  const r = rig();
  const g = r.gate();
  await hold(r);
  r.win.dispatchEvent(new Event('blur'));
  g.release();
  await r.clk.advance(0);
  assert.equal(r.open(), 0);
  assert.equal(phase(r), 'idle');
});

test('after the end the window is no longer watched: a later blur does nothing', async () => {
  const r = rig();
  await hold(r);
  r.speak(600);
  r.talk.release('pointer');
  await r.clk.advance(0);
  const before = r.views.length;
  r.win.dispatchEvent(new Event('blur'));
  assert.equal(r.views.length, before);
});

test('a microphone that is blocked shows why once and leaves nothing open', async () => {
  const r = rig();
  const g = r.gate();
  await hold(r);
  g.fail('NotAllowedError');
  await r.clk.advance(0);
  assert.equal(r.talk.view().note, WORDS.micBlocked);
  assert.equal(r.open(), 0);
  assert.equal(r.talk.release('pointer'), 'talk');
  await r.clk.advance(NOTE_MS);
  assert.equal(r.talk.view().note, null);
});

// ── Rule 4: empty, unavailable, failed ─────────────────────────────────

test('nothing heard: one quiet line, and nothing goes to her', async () => {
  const r = rig();
  r.heardAs.value = { status: 200, body: { text: '' } };
  await hold(r);
  r.speak(900);
  r.talk.release('pointer');
  await r.clk.advance(0);
  assert.equal(r.talk.view().note, "Didn't catch that.");
  assert.deepEqual(r.sent, []);
  assert.equal(phase(r), 'idle');
  await r.clk.advance(NOTE_MS);
  assert.equal(r.talk.view().note, null);
});

test('a clip too short to hold a sentence is not sent at all', async () => {
  const r = rig();
  await hold(r);
  r.speak(100);
  r.talk.release('pointer');
  await r.clk.advance(0);
  assert.equal(posts(r).length, 0);
  assert.equal(r.talk.view().note, "Didn't catch that.");
});

test('a 503 from the clip shows its reason and forgets that she could hear: the next press asks again', async () => {
  const r = rig();
  r.heardAs.value = { status: 503, body: { error: 'unavailable', reason: 'Hearing stopped for a moment. Try again.', fetching: false } };
  await hold(r);
  r.speak(900);
  r.talk.release('pointer');
  await r.clk.advance(0);
  assert.equal(r.talk.view().note, 'Hearing stopped for a moment. Try again.');
  assert.deepEqual(r.sent, []);
  assert.equal(asks(r).length, 1);

  await r.clk.advance(NOTE_MS);
  await hold(r);
  assert.equal(asks(r).length, 2, 'a yes is forgotten once the clip found otherwise');
  r.talk.cancel();
});

// ── Rule 2 (V1b): whether she can hear is asked before the microphone opens ──

const CANNOT = { status: 200, body: { hears: false, reason: 'Hearing needs 16 GB of memory, this Mac has 8 GB.', fetching: false } };
const FETCHING = { status: 200, body: { hears: false, reason: 'Fetching the hearing model (1.6 GB). Try again when it is done.', fetching: true } };

test('the question is asked at the hold, once, before any microphone', async () => {
  const r = rig();
  press(r);
  assert.equal(asks(r).length, 0, 'not at the pointer-down');
  await r.clk.advance(HOLD_MS);
  assert.equal(asks(r).length, 1);
  assert.equal(asks(r)[0].url, '/api/stt');
  assert.equal(r.asked, 1, 'she hears: the microphone opened after');
  r.talk.cancel();
});

test('a Mac that cannot hear: its reason from the first press, no microphone, no clip', async () => {
  const r = rig();
  r.hearsAs.value = CANNOT;
  await hold(r);
  assert.equal(r.asked, 0, 'no microphone was asked for');
  assert.equal(phase(r), 'refused');
  assert.equal(r.talk.view().note, 'Hearing needs 16 GB of memory, this Mac has 8 GB.');
  assert.equal(r.talk.release('pointer'), 'talk');
  await r.clk.advance(NOTE_MS);
  assert.equal(posts(r).length, 1, 'only the knock that starts a fetch, never a clip');
  assert.equal(r.open(), 0);
  assert.deepEqual(r.sent, []);
});

test('a fetch already on its way is told as it is and is not knocked on again', async () => {
  const r = rig();
  r.hearsAs.value = FETCHING;
  await hold(r);
  assert.equal(r.talk.view().note, FETCHING.body.reason);
  assert.equal(posts(r).length, 0);
  assert.equal(r.asked, 0);
  r.talk.release('pointer');
});

test('model not here and no fetch running: the press knocks once to start it, and shows what the knock answers', async () => {
  const r = rig();
  r.hearsAs.value = { status: 200, body: { hears: false, reason: 'The hearing model (1.6 GB) is not on this Mac yet.', fetching: false } };
  r.heardAs.value = { status: 503, body: { error: 'unavailable', reason: FETCHING.body.reason, fetching: true } };
  await hold(r);
  assert.equal(posts(r).length, 1);
  assert.equal(posts(r)[0].url, '/api/stt');
  assert.ok(!posts(r)[0].init.body?.includes('wav_b64'), 'a knock carries no sound');
  assert.equal(r.talk.view().note, FETCHING.body.reason);
  assert.equal(r.asked, 0);
  r.talk.release('pointer');
});

test('a no is never remembered: the next press asks again and listens once she can', async () => {
  const r = rig();
  r.hearsAs.value = FETCHING;
  await hold(r);
  r.talk.release('pointer');
  await r.clk.advance(NOTE_MS);
  r.hearsAs.value = { status: 200, body: { hears: true } };
  await hold(r);
  assert.equal(asks(r).length, 2);
  assert.equal(r.asked, 1);
  assert.equal(phase(r), 'listening');
  r.talk.cancel();
});

test('a yes is remembered for a short while only, then asked again', async () => {
  const r = rig();
  for (let n = 1; n <= 2; n++) {
    await hold(r);
    r.speak(600);
    r.talk.release('pointer');
    await r.clk.advance(NOTE_MS);
  }
  assert.equal(asks(r).length, 1, 'the second press did not ask');
  await r.clk.advance(HEARS_MS);
  await hold(r);
  assert.equal(asks(r).length, 2);
  r.talk.cancel();
});

test('let go while she is being asked: the microphone is never opened', async () => {
  const r = rig();
  r.hearsAs.value = 'wait';
  await hold(r);
  assert.equal(r.talk.release('pointer'), 'talk');
  r.answerHears({ status: 200, body: { hears: true } });
  await r.clk.advance(0);
  assert.equal(r.asked, 0);
  assert.equal(phase(r), 'idle');
});

test('the question failing is one line, and no microphone either', async () => {
  const r = rig();
  r.hearsAs.value = 'throw';
  await hold(r);
  assert.equal(r.talk.view().note, WORDS.failed);
  assert.equal(r.asked, 0);
  r.talk.release('pointer');
  r.hearsAs.value = { status: 403, body: { error: 'beyond_level' } };
  await hold(r);
  assert.equal(r.talk.view().note, WORDS.failed);
  assert.equal(r.asked, 0);
});

test('a 503 without a reason still says one thing', async () => {
  const r = rig();
  r.heardAs.value = { status: 503, body: {} };
  await hold(r);
  r.speak(900);
  r.talk.release('pointer');
  await r.clk.advance(0);
  assert.equal(r.talk.view().note, WORDS.cannotHear);
});

test('a failed request is one line and leaves nothing half-open', async () => {
  const r = rig();
  r.heardAs.value = 'throw';
  await hold(r);
  r.speak(900);
  r.talk.release('pointer');
  await r.clk.advance(0);
  assert.equal(r.talk.view().note, WORDS.failed);
  assert.equal(phase(r), 'idle');
  assert.equal(r.open(), 0);
  assert.deepEqual(r.sent, []);
});

test('her door refusing the line with a reason: that reason is the one line, and the heard words go', async () => {
  const r = rig();
  r.sendOk.value = false;
  r.sendReason.value = "A spoken line can't go to an app's chat. Type it there.";
  await hold(r);
  r.speak(900);
  r.talk.release('pointer');
  await r.clk.advance(0);
  assert.equal(r.talk.view().note, "A spoken line can't go to an app's chat. Type it there.");
  assert.equal(r.talk.view().heard, null);
  assert.equal(phase(r), 'idle');
});

test('her door refusing the line is one line too, and the heard words go', async () => {
  const r = rig();
  r.sendOk.value = false;
  await hold(r);
  r.speak(900);
  r.talk.release('pointer');
  await r.clk.advance(0);
  assert.equal(r.talk.view().note, WORDS.failed);
  assert.equal(r.talk.view().heard, null);
  assert.equal(phase(r), 'idle');
});

// ── A new press while the last is still being heard ────────────────────

test('a tap while hearing leaves the hearing alone', async () => {
  const r = rig();
  await hold(r);
  r.speak(900);
  r.talk.release('pointer');
  press(r);
  r.talk.release('pointer');
  await r.clk.advance(0);
  assert.deepEqual(r.sent, ['what is on today']);
});

test('a hold while hearing drops the old answer: only the new line goes', async () => {
  const r = rig();
  await hold(r);
  r.speak(900);
  r.talk.release('pointer');
  r.heardAs.value = { status: 200, body: { text: 'second' } };
  // The first request is already in flight; a new hold begins before its answer is read.
  press(r);
  await r.clk.advance(HOLD_MS);
  r.speak(900);
  r.talk.release('pointer');
  await r.clk.advance(0);
  assert.deepEqual(r.sent, ['second']);
});

// ── Keys ───────────────────────────────────────────────────────────────

test('the space bar starts a talk, except inside a field, a button, or as repeat', () => {
  const key = (over) => ({ code: 'Space', repeat: false, metaKey: false, ctrlKey: false, altKey: false, target: { tagName: 'DIV' }, ...over });
  assert.equal(isTalkKey(key({})), true);
  assert.equal(isTalkKey(key({ target: { tagName: 'INPUT' } })), false);
  assert.equal(isTalkKey(key({ target: { tagName: 'textarea' } })), false);
  assert.equal(isTalkKey(key({ target: { tagName: 'BUTTON' } })), false);
  assert.equal(isTalkKey(key({ target: { tagName: 'DIV', isContentEditable: true } })), false);
  assert.equal(isTalkKey(key({ repeat: true })), false);
  assert.equal(isTalkKey(key({ metaKey: true })), false);
  assert.equal(isTalkKey(key({ code: 'KeyA' })), false);
});

test('a key hold works as a pointer hold', async () => {
  const r = rig();
  await hold(r, 'key');
  r.speak(700);
  assert.equal(r.talk.release('key'), 'talk');
  await r.clk.advance(0);
  assert.deepEqual(r.sent, ['what is on today']);
  assert.equal(r.open(), 0);
});

// ── The locks ──────────────────────────────────────────────────────────

const repo = fileURLToPath(new URL('../..', import.meta.url));
function* files(dir) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) yield* files(p);
    // A `.d.ts` only describes shapes (ui/src/types/hold-to-talk.d.ts); it opens nothing.
    else if (/\.(ts|tsx|mts|js|jsx)$/.test(name) && !name.endsWith('.d.ts')) yield p;
  }
}
/** The files of the engine's shared folder and the UI's source holding `needle`, from the repo root. */
const withText = (needle) => ['shared', 'ui/src']
  .flatMap((dir) => [...files(join(repo, dir))])
  .filter((f) => readFileSync(f, 'utf8').includes(needle))
  .map((f) => relative(repo, f));
const shared = readFileSync(join(repo, 'shared/hold-to-talk.js'), 'utf8');

test('lock: one file opens the microphone, and it is shared/hold-to-talk.js', () => {
  assert.deepEqual(withText('getUserMedia'), ['shared/hold-to-talk.js']);
});

test('lock: one file carries a clip to the engine, and the one that records', () => {
  assert.deepEqual(withText('wav_b64'), ['shared/hold-to-talk.js']);
  assert.deepEqual(withText("'/api/stt'"), ['shared/hold-to-talk.js']);
});

test('lock: the control, the encoder and the recorder are one file that imports nothing', () => {
  assert.doesNotMatch(shared, /^\s*import[\s{'"]/m, 'a page loads this one file, alone');
  for (const gone of ['holdToTalk.mts', 'speechWav.mts', 'speechTap.ts']) {
    assert.deepEqual(readdirSync(join(repo, 'ui/src/lib')).filter((f) => f === gone), [], `${gone} is a second copy`);
  }
});

test('lock: the header a call names its wait in is the channel\'s and the UI policy\'s, word for word', () => {
  const literal = (file) => readFileSync(join(repo, file), 'utf8').match(/BUDGET_HEADER = '([^']+)'/)?.[1];
  const header = literal('shared/hold-to-talk.js');
  assert.equal(header, 'X-Linggen-Budget-Ms');
  assert.equal(literal('shared/channel.js'), header);
  assert.equal(literal('ui/src/lib/requestBudget.mts'), header);
});

test('lock: the one wait a hearing is given is a plain number the engine reads', () => {
  // src/server/api/stt_mlx.rs reads this line to hold its own waits inside it.
  assert.match(shared, /^export const HEAR_BUDGET_MS = [\d_]+;$/m);
});

// ── Review fixes ───────────────────────────────────────────────────────

test('rule 9: a press must last 350 ms to be a talk, so a slow click is still a click', async () => {
  assert.equal(HOLD_MS, 350);
  const r = rig();
  press(r);
  await r.clk.advance(300);
  assert.equal(r.talk.release('pointer'), 'tap');
  assert.equal(r.asked, 0);
});

test('rule 4: the audio context is made inside the press, once, and idled between talks', async () => {
  const r = rig();
  await hold(r);
  r.speak(600);
  r.talk.release('pointer');
  await r.clk.advance(0);
  await hold(r);
  r.talk.cancel();
  assert.equal(r.contexts, 1, 'one context for the page');
  assert.deepEqual(r.gestureWhenMade, [true], 'made in a press, never in the hold timer or after an await');
  assert.deepEqual(r.audio.filter((a) => a === 'resume').length, 2, 'woken at each press');
  assert.equal(r.audio.at(-1), 'suspend', 'idle once the microphone is off');
});

test('rule 4: a tap wakes the audio and idles it again, with no microphone', async () => {
  const r = rig();
  press(r);
  r.talk.release('pointer');
  assert.deepEqual(r.audio, ['resume', 'suspend']);
  assert.equal(r.asked, 0);
});

test('rule 4: a recorder that cannot load is one line, no microphone, and the next press tries again', async () => {
  const r = rig();
  r.workletFails = true;
  await hold(r);
  assert.equal(r.talk.view().note, WORDS.noMic);
  assert.equal(r.asked, 0);
  r.talk.release('pointer');
  r.workletFails = false;
  await hold(r);
  assert.equal(phase(r), 'listening');
  assert.equal(r.contexts, 2, 'a context whose recorder failed is made again');
  r.talk.cancel();
});

test('rule 4: the permission dialog took the press: told once to hold again, the microphone off', async () => {
  const r = rig();
  const g = r.gate();
  await hold(r);
  assert.equal(phase(r), 'opening');
  r.win.dispatchEvent(new Event('blur')); // the system's dialog takes focus
  g.release(); // the person clicks Allow
  await r.clk.advance(0);
  assert.equal(r.talk.view().note, WORDS.micAllowed);
  assert.equal(r.open(), 0);
  assert.equal(phase(r), 'idle');
  // Once: the next time the window leaves mid-open, nothing more is said.
  await r.clk.advance(NOTE_MS);
  const g2 = r.gate();
  await hold(r);
  r.win.dispatchEvent(new Event('blur'));
  g2.release();
  await r.clk.advance(0);
  assert.equal(r.talk.view().note, null);
  assert.equal(r.open(), 0);
});

test('rule 4: letting go early is not the dialog: nothing is said', async () => {
  const r = rig();
  const g = r.gate();
  await hold(r);
  r.talk.release('pointer');
  g.release();
  await r.clk.advance(0);
  assert.equal(r.talk.view().note, null);
  assert.equal(r.open(), 0);
});

test('rule 8: where the microphone can never be granted, a press stays a tap', async () => {
  const r = rig();
  r.offered = false;
  assert.equal(r.talk.offered(), false);
  press(r);
  await r.clk.advance(HOLD_MS * 2);
  assert.equal(r.talk.release('pointer'), 'tap');
  assert.equal(r.contexts, 0);
  assert.equal(r.asked, 0);
  assert.equal(r.fetched.length, 0);
  assert.equal(r.cuts, 0);
  assert.equal(phase(r), 'idle');
});

test('lock rule 4: no deprecated script processor, and the recorder is made from a blob, not fetched', () => {
  assert.deepEqual(withText('createScriptProcessor'), []);
  const tap = shared.slice(shared.indexOf("// ── The microphone's recorder"));
  assert.match(tap, /URL\.createObjectURL\(new Blob\(/);
  assert.doesNotMatch(tap, /fetch\(|XMLHttpRequest/);
});

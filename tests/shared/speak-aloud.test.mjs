// /shared/speak-aloud.js — a page reads a line aloud in the engine's voice: text in, one /api/tts
// request per clause, played in order, cut off by stop(). Everything it touches is a fake: no
// engine, no sound. Run: node --test tests/shared/*.test.mjs
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { CLAUSE_CHARS, clausesOf, createSpeakAloud, plainSpeech } from '../../shared/speak-aloud.js';

const flush = () => new Promise((r) => setImmediate(r));

/** A reader with a fake engine and a fake sound output. `r.ends()` ends what is playing, in order. */
function rig({ status = { speaks: true, natural: true }, failPost = false } = {}) {
  const r = { fetched: [], sources: [], contexts: 0, resumed: 0, gated: null };
  let clock = 0;
  const deps = {
    fetch: async (url, init) => {
      r.fetched.push({ url, init });
      if (init.method === 'GET') return { ok: true, status: 200, json: async () => status };
      if (r.gated) await r.gated;
      if (failPost) return { ok: false, status: 500, arrayBuffer: async () => new ArrayBuffer(0) };
      const text = JSON.parse(init.body).text;
      return { ok: true, status: 200, arrayBuffer: async () => new TextEncoder().encode(text).buffer };
    },
    newAudioContext: () => {
      r.contexts++;
      return {
        state: 'suspended',
        get currentTime() { return clock; },
        destination: {},
        resume: async () => { r.resumed++; },
        decodeAudioData: async (bytes) => ({ duration: 1, text: new TextDecoder().decode(bytes) }),
        createBufferSource: () => {
          const src = { buffer: null, started: null, stopped: false, onended: null, connect() { /* out */ }, start(at) { src.started = at; }, stop() { src.stopped = true; } };
          r.sources.push(src);
          return src;
        },
      };
    },
  };
  const voice = createSpeakAloud(deps);
  return Object.assign(r, {
    voice,
    gate() { let open; r.gated = new Promise((res) => { open = res; }); return () => { r.gated = null; open(); }; },
    /** The sources that have been given to the output and not yet ended or stopped. */
    live: () => r.sources.filter((s) => s.started !== null && !s.stopped && !s.ended),
    endAll() { for (const s of r.sources) if (s.started !== null && !s.stopped && !s.ended) { s.ended = true; s.onended?.(); } },
  });
}
const posts = (r) => r.fetched.filter((f) => f.init.method === 'POST');

test('status asks, never speaks: one GET, no POST, no sound context', async () => {
  const r = rig({ status: { speaks: true, natural: false } });
  assert.deepEqual(await r.voice.status(), { speaks: true, natural: false, reason: null });
  assert.equal(r.fetched.length, 1);
  assert.deepEqual([r.fetched[0].url, r.fetched[0].init.method], ['/api/tts', 'GET']);
  assert.equal(r.contexts, 0, 'asking makes no audio context');
  const no = rig({ status: { speaks: false, natural: false, reason: 'No voice here.' } });
  assert.deepEqual(await no.voice.status(), { speaks: false, natural: false, reason: 'No voice here.' });
});

test('status of an engine that cannot be reached is a failed ask, not a no', async () => {
  const voice = createSpeakAloud({ fetch: async () => { throw new Error('down'); }, newAudioContext: () => null });
  assert.deepEqual(await voice.status(), { speaks: false, natural: false, reason: null, failed: true });
  const refused = createSpeakAloud({ fetch: async () => ({ ok: false, status: 500, json: async () => ({}) }), newAudioContext: () => null });
  assert.equal((await refused.status()).failed, true);
});

test('a line is asked for clause by clause, in order, and played back to back', async () => {
  const r = rig();
  const done = r.voice.say('First one. Second one. Third one.');
  await flush(); await flush(); await flush();
  assert.deepEqual(posts(r).map((p) => JSON.parse(p.init.body).text), ['First one. Second one. Third one.'], 'short clauses are joined into one request');
  r.endAll();
  assert.deepEqual(await done, { spoken: true });
  assert.equal(r.voice.speaking(), false);
});

test('a long line is several requests; the next is asked while the first plays; queue times add up', async () => {
  const r = rig();
  const sentence = (n) => `Sentence ${n} ` + 'x'.repeat(CLAUSE_CHARS - 20) + '.';
  const done = r.voice.say([1, 2, 3].map(sentence).join(' '));
  for (let i = 0; i < 12; i++) await flush();
  assert.equal(posts(r).length, 3);
  assert.deepEqual(r.sources.map((s) => s.started), [0, 1, 2], 'each clause starts when the one before it ends');
  assert.equal(r.voice.speaking(), true);
  r.endAll();
  assert.deepEqual(await done, { spoken: true });
  assert.equal(r.voice.speaking(), false);
});

test('stop() cuts what sounds, asks for nothing more, and ends the line', async () => {
  const r = rig();
  const sentence = (n) => `Sentence ${n} ` + 'x'.repeat(CLAUSE_CHARS - 20) + '.';
  const open = r.gate();
  const done = r.voice.say([1, 2, 3].map(sentence).join(' '));
  await flush();
  r.voice.stop();
  open();
  assert.deepEqual(await done, { spoken: false });
  for (let i = 0; i < 5; i++) await flush();
  assert.equal(r.sources.length, 0, 'nothing was played after the stop');
  assert.equal(r.voice.speaking(), false);

  const playing = r.voice.say('One line.');
  for (let i = 0; i < 6; i++) await flush();
  assert.equal(r.live().length, 1);
  r.voice.stop();
  assert.ok(r.sources[0].stopped, 'the sound that was playing is stopped');
  assert.deepEqual(await playing, { spoken: false });
});

test('a newer line replaces the one being said', async () => {
  const r = rig();
  const first = r.voice.say('The old line.');
  for (let i = 0; i < 6; i++) await flush();
  const second = r.voice.say('The new line.');
  for (let i = 0; i < 6; i++) await flush();
  assert.ok(r.sources[0].stopped);
  assert.equal((await first).spoken, false);
  r.endAll();
  assert.deepEqual(await second, { spoken: true });
});

test('a voice that fails says nothing and does not throw', async () => {
  const r = rig({ failPost: true });
  assert.deepEqual(await r.voice.say('Hello there.'), { spoken: false });
  assert.equal(r.voice.speaking(), false);
  assert.deepEqual(await r.voice.say('   '), { spoken: false }, 'nothing to say is not a request');
  assert.equal(posts(r).length, 1);
});

test('prime() wakes the sound inside a gesture, once made it is the same context', async () => {
  const r = rig();
  r.voice.prime();
  r.voice.prime();
  assert.equal(r.contexts, 1);
  assert.equal(r.resumed, 2);
});

test('subscribers hear when she starts and stops', async () => {
  const r = rig();
  const seen = [];
  r.voice.subscribe(() => seen.push(r.voice.speaking()));
  const done = r.voice.say('Hi.');
  for (let i = 0; i < 6; i++) await flush();
  r.endAll();
  await done;
  assert.deepEqual(seen, [true, false]);
});

test('what is read is the words: markdown marks, code and links are not spoken', () => {
  assert.equal(plainSpeech('**Nice** to meet you, _Alex_!'), 'Nice to meet you, Alex!');
  assert.equal(plainSpeech('# Today\n- one\n- two'), 'Today\none\ntwo');
  assert.equal(plainSpeech('See [the page](https://x.test/a) now. ```js\nlet a = 1;\n``` Done.'), 'See the page now.   Done.'.replace(/ {2,}/g, ' '));
  assert.equal(plainSpeech(null), '');
});

test('a line is cut at sentence ends, not inside a number or a name; Chinese ends at 。', () => {
  assert.deepEqual(clausesOf('It costs 3.5 dollars, i.e. little. Fine?'), ['It costs 3.5 dollars, i.e. little. Fine?']);
  const long = Array.from({ length: 12 }, (_, i) => `Sentence number ${i + 1} is here.`).join(' ');
  const clauses = clausesOf(long);
  assert.ok(clauses.length > 1 && clauses.every((c) => c.length <= CLAUSE_CHARS), JSON.stringify(clauses.map((c) => c.length)));
  assert.equal(clauses.join(' '), long, 'nothing is lost or added');
  assert.deepEqual(clausesOf('你好。我是灵。'), ['你好。我是灵。'], 'short Chinese clauses are joined with no space');
  const noBreaks = 'y'.repeat(CLAUSE_CHARS * 2 + 10);
  assert.ok(clausesOf(noBreaks).every((c) => c.length <= CLAUSE_CHARS));
  assert.equal(clausesOf('  ').length, 0);
});

// ── Locks ──

const repo = join(fileURLToPath(import.meta.url), '../../..');
const source = readFileSync(join(repo, 'shared/speak-aloud.js'), 'utf8');

test('lock: the reader imports nothing, opens no microphone and asks no other route than /api/tts', () => {
  assert.doesNotMatch(source, /^\s*import[\s{'"]/m, 'a page loads this one file, alone');
  assert.doesNotMatch(source, /getUserMedia/);
  const urls = [...source.matchAll(/'(\/api\/[^']*)'/g)].map((m) => m[1]);
  assert.deepEqual([...new Set(urls)], ['/api/tts']);
});

test('lock: both players say where the other one is', () => {
  const companion = readFileSync(join(repo, 'ui/src/lib/eventHandlers/yinyue.ts'), 'utf8');
  assert.match(source, /eventHandlers\/yinyue\.ts/);
  assert.match(companion, /shared\/speak-aloud\.js/);
});

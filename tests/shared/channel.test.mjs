// /shared/channel.js — a skill page's data path. Every engine call rides a
// data channel (a frame above that holds one, or the page's own channel
// frame); a call made before it is up waits for it; nothing goes over plain
// HTTP. Run: node --test tests/shared/*.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import '../../shared/channel.js';

const {
  createChannel, routeOf, NotConnectedError, toResponse,
} = globalThis.LinggenChannel;

const PAGE = 'http://engine/apps/dice/index.html';

/** A host as the engine's UI offers it: records what it is asked. */
function fakeHost(reply = { status: 200, content_type: 'application/json', body: '{"ok":true}' }) {
  const asked = [];
  return {
    asked,
    isOpen: () => true,
    async request(req) {
      asked.push(req);
      return typeof reply === 'function' ? reply(req) : reply;
    },
  };
}

/** A page: `parent` is the frame above (none: standing alone). */
function fakeWin(parent) {
  const win = { location: { href: PAGE } };
  win.parent = parent || win;
  return win;
}

const noMeta = { querySelector: () => null };

/** A channel whose own frame comes up when `frameUp()` is called. */
function pageChannel({ win = fakeWin(), doc = noMeta, waitMs = 2000 } = {}) {
  const http = [];
  const host = fakeHost();
  let up;
  const frame = new Promise((resolve) => (up = resolve));
  let opened = 0;
  const channel = createChannel({
    win,
    doc,
    waitMs,
    fetch: async (input) => {
      http.push(String(input));
      return new Response('static');
    },
    openFrame: () => {
      opened += 1;
      return frame;
    },
  });
  return { channel, http, host, opened: () => opened, frameUp: () => up({ host, alive: () => true }) };
}

test('engine calls ride the channel; static files and signaling stay HTTP', () => {
  assert.deepEqual(routeOf('/api/models', undefined, PAGE), { method: 'GET', path: '/api/models' });
  assert.deepEqual(routeOf('http://engine/api/x?y=1', { method: 'post' }, PAGE), { method: 'POST', path: '/api/x?y=1' });
  assert.deepEqual(routeOf('/apps/dice/capability/Roll', { method: 'POST' }, PAGE), { method: 'POST', path: '/apps/dice/capability/Roll' });
  assert.equal(routeOf('/apps/dice/data/table.json', undefined, PAGE), null);
  assert.equal(routeOf('data/table.json', undefined, PAGE), null);
  assert.equal(routeOf('/api/rtc/whip', { method: 'POST' }, PAGE), null);
  assert.equal(routeOf('https://hn.algolia.com/api/v1/items/1', undefined, PAGE), null);
});

test('a call made before the channel is up waits for it, then goes over it — never HTTP', async () => {
  const { channel, http, host, frameUp } = pageChannel();
  const early = channel.fetch('/api/models');
  await new Promise((r) => setTimeout(r, 20));
  assert.deepEqual(host.asked, []); // nothing went: there is no channel yet
  frameUp();
  const res = await early;
  assert.deepEqual(await res.json(), { ok: true });
  assert.deepEqual(host.asked.map((r) => `${r.method} ${r.path}`), ['GET /api/models']);
  assert.deepEqual(http, []);
  // A static file is the page's own: plain HTTP.
  await channel.fetch('data/table.json');
  assert.deepEqual(http, ['data/table.json']);
});

test('a channel that never comes up fails the call with a clear error, not HTTP', async () => {
  const { channel, http } = pageChannel({ waitMs: 50 });
  await assert.rejects(channel.fetch('/api/models'), (e) => {
    assert.ok(e instanceof NotConnectedError);
    assert.match(e.message, /Not connected to Linggen: GET \/api\/models waited/);
    return true;
  });
  assert.deepEqual(http, []);
});

test('a page framed by one that holds a channel asks it — no channel frame of its own', async () => {
  const above = fakeHost();
  const top = { __linggenChannel: above };
  top.parent = top;
  const { channel, opened } = pageChannel({ win: fakeWin(top) });
  await channel.fetch('/api/skill-sessions?skill=dice');
  assert.deepEqual(above.asked.map((r) => r.path), ['/api/skill-sessions?skill=dice']);
  assert.equal(opened(), 0);
});

test("a frame of another origin above is not the page's channel", async () => {
  const foreign = {};
  Object.defineProperty(foreign, '__linggenChannel', { get() { throw new Error('cross-origin'); } });
  foreign.parent = foreign;
  const { channel, host, frameUp, opened } = pageChannel({ win: fakeWin(foreign) });
  frameUp();
  await channel.fetch('/api/models');
  assert.equal(opened(), 1);
  assert.equal(host.asked.length, 1);
});

test('bytes come back whole: a base64 reply is the bytes it was', async () => {
  const wav = new Uint8Array([0x52, 0x49, 0x46, 0x46, 0xff, 0xfe, 0x00, 0x80]);
  const body = Buffer.from(wav).toString('base64');
  const res = toResponse({ status: 200, content_type: 'audio/wav', body, body_encoding: 'base64' });
  assert.equal(res.headers.get('content-type'), 'audio/wav');
  assert.deepEqual(new Uint8Array(await res.arrayBuffer()), wav);
  assert.equal(toResponse({ status: 204, body: '' }).status, 204);
});

test('a JSON body is named JSON; a body the channel cannot carry is an error', async () => {
  const { channel, host, frameUp } = pageChannel();
  frameUp();
  await channel.fetch('/api/bash', { method: 'POST', body: JSON.stringify({ command: 'ls' }) });
  assert.deepEqual(host.asked[0], {
    method: 'POST', path: '/api/bash', body: '{"command":"ls"}', contentType: 'application/json',
  });
  await assert.rejects(
    channel.fetch('/api/upload', { method: 'POST', body: new Blob(['x']) }),
    /carries text and JSON bodies only/,
  );
});

test('a reading (wait: false) is skipped while the channel is down, and starts bringing it up', async () => {
  const { channel, opened, frameUp, host } = pageChannel();
  await assert.rejects(
    channel.request({ method: 'POST', path: '/api/presence', body: '{}', wait: false }),
    NotConnectedError,
  );
  assert.equal(opened(), 1);
  frameUp();
  await new Promise((r) => setTimeout(r, 0));
  await channel.request({ method: 'POST', path: '/api/presence', body: '{}', wait: false });
  assert.equal(host.asked.length, 1);
});

test("the host's own not-connected error reaches the page as one", async () => {
  const { channel, host, frameUp } = pageChannel();
  host.request = async () => {
    const e = new Error('Not connected to Linggen: GET /api/models waited 20 s for the data channel');
    e.name = 'NotConnectedError';
    throw e;
  };
  frameUp();
  await assert.rejects(channel.fetch('/api/models'), NotConnectedError);
});

test('a removed channel frame is opened again', async () => {
  let alive = true;
  const host = fakeHost();
  let opened = 0;
  const channel = createChannel({
    win: fakeWin(),
    doc: noMeta,
    fetch: async () => new Response(''),
    openFrame: async () => {
      opened += 1;
      return { host, alive: () => alive };
    },
  });
  await channel.fetch('/api/models');
  alive = false; // the page rewrote itself and took the frame with it
  await channel.fetch('/api/models');
  assert.equal(opened, 2);
});

test('remotely, the tunnel worker already carries every call: fetch goes as it is', async () => {
  const doc = { querySelector: (q) => (q.includes('linggen-instance') ? {} : null) };
  const { channel, http, opened } = pageChannel({ doc });
  await channel.fetch('/api/models');
  assert.deepEqual(http, ['/api/models']);
  assert.equal(opened(), 0);
});

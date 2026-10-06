// node --test ui/tests/*.test.mts (Node strips the types).
// The UI's data rides the data channel only: a call made before the channel
// is open waits for it, a channel that never opens fails the call with a
// clear error, and bytes come back intact.
import { test, mock } from 'node:test';
import assert from 'node:assert/strict';
import {
  CONNECT_WAIT_MS, NotConnectedError, channelBody, createGate, ridesChannel, toPath, toResponse,
} from '../src/lib/channelFetch.mts';

test('a call made before the channel opens waits, and goes once it opens', async () => {
  const gate = createGate();
  let sent = false;
  const call = gate.whenOpen('POST /api/chat').then(() => { sent = true; });
  await Promise.resolve();
  assert.equal(sent, false);
  gate.set(true);
  await call;
  assert.equal(sent, true);
});

test('an open channel lets a call through at once', async () => {
  const gate = createGate();
  gate.set(true);
  assert.equal(gate.isOpen(), true);
  await gate.whenOpen('GET /api/status');
});

test('every waiting call goes when the channel opens, in the order they came', async () => {
  const gate = createGate();
  const order: string[] = [];
  const calls = ['GET /api/account', 'GET /api/status', 'POST /api/chat'].map((what) =>
    gate.whenOpen(what).then(() => order.push(what)));
  gate.set(true);
  await Promise.all(calls);
  assert.deepEqual(order, ['GET /api/account', 'GET /api/status', 'POST /api/chat']);
});

test('a dropped channel holds new calls until it is back', async () => {
  const gate = createGate();
  gate.set(true);
  gate.set(false); // reconnecting
  let sent = false;
  const call = gate.whenOpen('GET /api/status').then(() => { sent = true; });
  await Promise.resolve();
  assert.equal(sent, false);
  gate.set(true);
  await call;
  assert.equal(sent, true);
});

test('a channel that never opens fails the call with a clear error, not silence', async () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  try {
    const gate = createGate();
    const call = gate.whenOpen('GET /api/status');
    mock.timers.tick(CONNECT_WAIT_MS);
    await assert.rejects(call, (e: unknown) =>
      e instanceof NotConnectedError && /GET \/api\/status waited 20 s for the data channel/.test(e.message));
    // A late open no longer releases the call that already failed.
    gate.set(true);
  } finally {
    mock.timers.reset();
  }
});

test('a call that went is not failed by its timer afterwards', async () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  try {
    const gate = createGate();
    const call = gate.whenOpen('GET /api/status', 1000);
    gate.set(true);
    await call;
    mock.timers.tick(5000); // no stray rejection
  } finally {
    mock.timers.reset();
  }
});

test('the API rides the channel; signaling and static files do not', () => {
  for (const p of ['/api/presence', '/api/tts', '/api/yinyue/chat', '/api/status?x=1', '/apps/dice/data.json', '/assets/a.js', '/logo.svg']) {
    assert.equal(ridesChannel(p), true, p);
  }
  for (const p of ['/api/rtc/whip', '/api/rtc/token', '/', '/yinyue.vrm', '/anim/actions.json', 'https://example.com/api/x']) {
    assert.equal(ridesChannel(p), false, p);
  }
});

test('a same-origin absolute URL routes as its path', () => {
  assert.equal(toPath('https://linggen.dev/api/workspace/state?a=1', 'https://linggen.dev'), '/api/workspace/state?a=1');
  assert.equal(toPath('https://linggen.dev.evil/api/x', 'https://linggen.dev'), 'https://linggen.dev.evil/api/x');
  assert.equal(toPath('/api/x', 'https://linggen.dev'), '/api/x');
});

test('a JSON body rides parsed; other bodies as given', () => {
  assert.deepEqual(channelBody('{"text":"hi"}', 'application/json'), { text: 'hi' });
  assert.equal(channelBody('plain', 'text/plain'), 'plain');
  assert.equal(channelBody('not json', 'application/json'), 'not json');
  assert.equal(channelBody(undefined, 'application/json'), undefined);
});

test('a byte reply (Yinyue\'s WAV) comes back intact', async () => {
  const wav = new Uint8Array([0x52, 0x49, 0x46, 0x46, 0xff, 0xfe, 0x00, 0x80]);
  const resp = toResponse({
    status: 200, content_type: 'audio/wav', body_encoding: 'base64', body: Buffer.from(wav).toString('base64'),
  });
  assert.equal(resp.headers.get('content-type'), 'audio/wav');
  assert.deepEqual(new Uint8Array(await resp.arrayBuffer()), wav);
});

test('a text reply keeps its status and type; a bodiless status has no body', async () => {
  const resp = toResponse({ status: 404, content_type: 'application/json', body: '{"error":"gone"}' });
  assert.equal(resp.status, 404);
  assert.deepEqual(await resp.json(), { error: 'gone' });
  assert.equal(toResponse({ status: 204, body: '' }).status, 204);
});

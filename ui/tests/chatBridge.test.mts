// node --test ui/tests/*.test.mts
// /shared/chat-bridge.js with a stub DOM: a page's onStreamToken/onStreamEnd
// hear its own agent's turns only, unless it opts into guest streams.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

type Listener = (e: unknown) => void;
const windowListeners: Listener[] = [];

function fakeElement() {
  const listeners: Record<string, Listener[]> = {};
  const el = {
    style: {} as Record<string, string>,
    contentWindow: { postMessage() {} },
    addEventListener(type: string, fn: Listener) { (listeners[type] ??= []).push(fn); },
    appendChild(child: ReturnType<typeof fakeElement>) { setTimeout(() => child.fire('load'), 0); },
    remove() {},
    fire(type: string) { for (const fn of listeners[type] ?? []) fn({}); },
  };
  return el;
}

const g = globalThis as Record<string, unknown>;
g.location = { origin: 'http://engine', pathname: '/apps/demo/', href: 'http://engine/apps/demo/' };
g.document = {
  querySelector: () => null,
  querySelectorAll: () => [],
  createElement: () => fakeElement(),
  addEventListener() {},
  hasFocus: () => false,
  visibilityState: 'hidden',
};
g.window = { ...g, addEventListener: (t: string, fn: Listener) => { if (t === 'message') windowListeners.push(fn); }, removeEventListener() {} };
g.fetch = async () => ({ ok: true, json: async () => ({ id: 'sess-1' }) });
g.setInterval = () => 0;

// The bridge imports '/shared/api.js' by its served path; point it at the file.
const shared = new URL('../../shared/', import.meta.url);
const dir = mkdtempSync(join(tmpdir(), 'bridge-'));
const src = readFileSync(new URL('chat-bridge.js', shared), 'utf8')
  .replace("from '/shared/api.js'", `from '${new URL('api.js', shared).href}'`);
writeFileSync(join(dir, 'chat-bridge.mjs'), src);
const { mount } = await import(pathToFileURL(join(dir, 'chat-bridge.mjs')).href);

async function mountWith(opts: Record<string, unknown>) {
  const heard: { kind: string; text: string; agent?: string }[] = [];
  const el = fakeElement();
  const before = windowListeners.length;
  let iframeWin: unknown;
  el.appendChild = (child: ReturnType<typeof fakeElement>) => { iframeWin = child.contentWindow; setTimeout(() => child.fire('load'), 0); };
  await mount(el, {
    skillName: 'demo', agentId: 'ling', sessionId: 'sess-1',
    onStreamToken: (text: string, info?: { agent: string }) => heard.push({ kind: 'token', text, agent: info?.agent }),
    onStreamEnd: (text: string, info?: { agent: string }) => heard.push({ kind: 'end', text, agent: info?.agent }),
    ...opts,
  });
  const mine = windowListeners.slice(before);
  const emit = (event: string, payload: unknown) => {
    for (const fn of mine) fn({ data: { type: 'linggen-skill-event', event, payload }, source: iframeWin, origin: 'http://engine' });
  };
  return { heard, emit };
}

test("a guest's tokens and turn end never reach the page's turn handlers", async () => {
  const { heard, emit } = await mountWith({});
  emit('stream_token', { text: 'Reck', agent: 'ling', own: true });
  emit('stream_token', { text: '我在', agent: 'yinyue', own: false });
  emit('stream_end', { text: '我在。', agent: 'yinyue', own: false });
  emit('stream_token', { text: 'oning', agent: 'ling', own: true });
  emit('stream_end', { text: 'Reckoning', agent: 'ling', own: true });
  assert.deepEqual(heard.map((h) => [h.kind, h.text]), [['token', 'Reck'], ['token', 'Reckoning'], ['end', 'Reckoning']]);
});

test('a page that opts in hears her lines too, each named', async () => {
  const { heard, emit } = await mountWith({ guestStreams: true });
  emit('stream_token', { text: '我在', agent: 'yinyue', own: false });
  emit('stream_end', { text: '我在。', agent: 'yinyue', own: false });
  assert.deepEqual(heard, [{ kind: 'token', text: '我在', agent: 'yinyue' }, { kind: 'end', text: '我在', agent: 'yinyue' }]);
});

test('an older embed that names no agent streams to the page as before', async () => {
  const { heard, emit } = await mountWith({});
  emit('stream_token', { text: 'hi' });
  emit('stream_end', { text: 'hi' });
  assert.deepEqual(heard.map((h) => h.kind), ['token', 'end']);
});

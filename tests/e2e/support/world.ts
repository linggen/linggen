// A test's own world: tests/world started as a child process (the same
// world-builder the Rust system tests use), stopped when the test ends.
// A failed test keeps the world's root (the engine and ling-mem logs) and
// attaches the engine log and the world's own stderr.
//
// Every test also checks, when it ends, that the page sent nothing over
// plain HTTP beyond what loads the page and signals WebRTC (`HTTP_ALLOWED`):
// the UI's data rides the data channel.
import { test as base, expect, type Page } from '@playwright/test';
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { createInterface } from 'node:readline';
import { readFileSync } from 'node:fs';
import path from 'node:path';

/** What the fake model answers, call by call, per member. */
export type Reply = { text: string } | { tool: string; args: Record<string, unknown> };
export type Scenario = {
  /** tests/fixtures/<fixture>; `home` when unset. */
  fixture?: string;
  /** A true first install: no fixture, no config folder at all (the engine's
   *  own defaults; its ling-mem named only by `LING_MEM_URL`). */
  noConfig?: boolean;
  scripts?: { ling?: Reply[]; yinyue?: Reply[] };
};

export type ModelCall = { agent: string | null; matched: boolean; model: string; path: string };

export type World = {
  url: string;
  root: string;
  work: string;
  linggenHome: string;
  /** Every call the fake model got so far. */
  calls(): Promise<ModelCall[]>;
  engineLog(): string;
};

/** The real Linggen ports — a world never uses them (as tests/system's guard). */
const REAL_PORTS = ['9527', '9528'];
const ROOT_BASE = '/var/tmp/linggen-system-tests/';

function guard(hello: { url: string; root: string }) {
  const port = new URL(hello.url).port;
  if (REAL_PORTS.includes(port)) throw new Error(`HERMETIC GUARD: world on real port ${port}`);
  const root = hello.root.replace(/^\/private/, '');
  if (!root.startsWith(ROOT_BASE)) throw new Error(`HERMETIC GUARD: world root ${hello.root}`);
}

type Running = {
  world: World;
  child: ChildProcessWithoutNullStreams;
  /** Settles when the world process is gone (at once if it already is). */
  exited: Promise<void>;
  stderr(): string;
};

async function startWorld(scenario: Scenario): Promise<Running> {
  const bin = process.env.LINGGEN_WORLD_BIN;
  if (!bin) throw new Error('LINGGEN_WORLD_BIN unset — run through playwright (global setup builds it)');
  const child = spawn(bin, [JSON.stringify(scenario)], { stdio: ['pipe', 'pipe', 'pipe'] });
  let stderr = '';
  child.stderr.on('data', (d) => (stderr += d));
  // One exit listener for the world's whole life: it fails every line still
  // awaited, and any asked for later.
  const waiting: { resolve: (line: string) => void; reject: (e: Error) => void }[] = [];
  let gone: Error | null = null;
  const exited = new Promise<void>((resolve) => {
    child.once('exit', (code, signal) => {
      gone = new Error(`world exited (${code ?? signal}):\n${stderr}`);
      for (const w of waiting.splice(0)) w.reject(gone);
      resolve();
    });
  });
  const lines = createInterface({ input: child.stdout });
  lines.on('line', (line) => waiting.shift()?.resolve(line));
  const nextLine = () =>
    new Promise<string>((resolve, reject) => {
      if (gone) return reject(gone);
      waiting.push({ resolve, reject });
    });

  const hello = JSON.parse(await nextLine());
  guard(hello);
  const world: World = {
    url: hello.url,
    root: hello.root,
    work: hello.work,
    linggenHome: hello.linggen_home,
    async calls() {
      const reply = nextLine();
      child.stdin.write('calls\n');
      return JSON.parse(await reply).calls;
    },
    engineLog() {
      try {
        return readFileSync(path.join(hello.root, 'engine.log'), 'utf8');
      } catch {
        return '';
      }
    },
  };
  return { world, child, exited, stderr: () => stderr };
}

/** How long a world gets to stop (every process, its root removed) before
 *  it is killed — its own watchdogs then stop the engine and ling-mem. */
const STOP_GRACE_MS = 20_000;

async function stopWorld({ child, exited }: Running) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  child.stdin.end();
  const timer = setTimeout(() => child.kill('SIGKILL'), STOP_GRACE_MS);
  await exited;
  clearTimeout(timer);
}

/** Wraps RTCPeerConnection so a test can read the transport's real state. */
function watchPeerConnections() {
  const Orig = window.RTCPeerConnection;
  const pcs: RTCPeerConnection[] = [];
  (window as unknown as { __pcs: RTCPeerConnection[] }).__pcs = pcs;
  window.RTCPeerConnection = class extends Orig {
    constructor(...args: ConstructorParameters<typeof RTCPeerConnection>) {
      super(...args);
      pcs.push(this);
    }
  } as typeof RTCPeerConnection;
}

/** Caps animation frames at 10 a second (opt in: `throttleFrames`).
 *  Yinyue's avatar renders WebGL in software in headless Chromium at 60 fps
 *  — on a busy machine that starves the page's main thread (a fill took
 *  28 s). Nothing a test reads depends on frame rate. */
function throttleAnimationFrames() {
  window.requestAnimationFrame = (cb) =>
    window.setTimeout(() => cb(performance.now()), 100) as unknown as number;
  window.cancelAnimationFrame = (id) => window.clearTimeout(id);
}

type Fixtures = {
  scenario: Scenario;
  /** Cap the page's animation frames (see `throttleAnimationFrames`). */
  throttleFrames: boolean;
  world: World;
  /** The requests the page sent over plain HTTP (not the data channel) since
   *  its last load, as `METHOD /path` — a reload starts the list again. */
  httpRequests: string[];
};

export const test = base.extend<Fixtures>({
  scenario: [{}, { option: true }],
  throttleFrames: [false, { option: true }],

  world: async ({ scenario }, use, testInfo) => {
    const running = await startWorld(scenario);
    const { world, child } = running;
    await use(world);
    const failed = testInfo.status !== testInfo.expectedStatus;
    const alive = child.exitCode === null && child.signalCode === null;
    if (failed || !alive) {
      await testInfo.attach('engine.log', { body: world.engineLog(), contentType: 'text/plain' });
      await testInfo.attach('world.stderr', { body: running.stderr(), contentType: 'text/plain' });
    }
    if (failed && alive) child.stdin.write('keep\n');
    await stopWorld(running);
    if (!alive) throw new Error(`the world stopped during the test:\n${running.stderr()}`);
  },

  httpRequests: async ({}, use) => {
    await use([]);
  },

  page: async ({ page, httpRequests, throttleFrames }, use) => {
    await page.addInitScript(watchPeerConnections);
    if (throttleFrames) await page.addInitScript(throttleAnimationFrames);
    const all: string[] = [];
    page.on('request', (req) => {
      if (req.isNavigationRequest() && req.frame() === page.mainFrame()) httpRequests.length = 0;
      const r = describe(req.method(), req.url(), page.url());
      httpRequests.push(r);
      all.push(r);
    });
    await use(page);
    expectNoHttpData(all);
  },
});

export { expect };

/** The UI's WebRTC transport is up: a peer connection reached `connected`
 *  after signaling over /api/rtc. */
export async function expectWebRtc(page: Page, httpRequests: string[]) {
  await expect
    .poll(() =>
      page.evaluate(() =>
        ((window as unknown as { __pcs?: RTCPeerConnection[] }).__pcs ?? []).some(
          (pc) => pc.connectionState === 'connected',
        ),
      ),
    )
    .toBe(true);
  expect(httpRequests.some((r) => r.includes('/api/rtc/'))).toBe(true);
}

/** `METHOD /path` for the page's own origin; `METHOD <url>` elsewhere —
 *  another host, or a `blob:`/`data:` URL (in-page, never on the wire). */
function describe(method: string, url: string, pageUrl: string): string {
  const u = new URL(url);
  if (u.protocol !== 'http:' && u.protocol !== 'https:') return `${method} ${u.protocol}`;
  const origin = new URL(pageUrl).origin; // 'null' before the first load
  return u.origin === origin || origin === 'null' ? `${method} ${u.pathname}` : `${method} ${u.href}`;
}

/** What may go over plain HTTP: the page and its static files, and the
 *  WebRTC signaling that brings the data channel up. */
const HTTP_ALLOWED: RegExp[] = [
  /^GET \/$/, // the page (`/?session=…` too)
  /^GET \/assets\//, // its scripts, styles, lazy chunks
  /^GET \/(yinyue\.vrm|logo\.svg|favicon\.ico)$/, // static files from ui/public
  /^GET \/anim\//, // Yinyue's animations
  /^(GET|POST|PATCH|DELETE) \/api\/rtc\//, // signaling
  /^GET (blob|data):$/, // in-page URLs: no request leaves the page
];

/** Nothing went over plain HTTP but `HTTP_ALLOWED` — the UI's data rode
 *  the data channel, before it connected too (a call waits for it). No
 *  exceptions: a data call over HTTP fails the test. Checked for every test
 *  when it ends. */
export function expectNoHttpData(requests: string[]) {
  const stray = requests.filter((r) => !HTTP_ALLOWED.some((re) => re.test(r)));
  expect([...new Set(stray)], 'requests sent over plain HTTP').toEqual([]);
}

/** The session's turn is over: the chat box no longer shows it busy (the
 *  engine's Idle, and nothing left pending). A count taken after this sees
 *  every row the turn will ever show. */
export async function expectTurnOver(page: Page) {
  await expect(chat(page).getByTestId('chat-input')).toHaveAttribute('data-busy', 'false');
}

/** Every model call so far was one a test scripted (none fell to the
 *  catch-all), and they came from `agents`, in order. */
export async function expectScriptedCalls(world: World, agents: string[]) {
  const calls = await world.calls();
  expect(calls.filter((c) => !c.matched || c.agent === 'unscripted')).toEqual([]);
  expect(calls.map((c) => c.agent)).toEqual(agents);
}

/** The chat column (transcript + input), without the side panels. */
export function chat(page: Page) {
  return page.locator('main');
}

/** Type a message in the chat box and send it with Enter. */
export async function send(page: Page, text: string) {
  const box = page.getByPlaceholder(/^Message/);
  await box.fill(text);
  await box.press('Enter');
}

/** Text the chat shows — visible copies only (the page keeps hidden copies
 *  of some rows, e.g. while a turn settles). */
export function shown(page: Page, text: string | RegExp) {
  return chat(page).getByText(text).filter({ visible: true });
}

/** A request the page asks over the data channel, by method and path. */
export type Asked = { method: string; path: string };

/** What to hold: replies to the requests `asked` names, and with `events`
 *  everything a session's own channel (`sess-<id>`) pushes. */
export type Hold = { asked?: Asked[]; events?: boolean };

/** Holds replies and pushed events (see `Hold`) until the test releases
 *  them — so a test can make one land after another, or after the page
 *  moved on (an old session's load after New chat), or read a turn's events
 *  in the order they came. Install with `page.addInitScript(holdReplies,
 *  hold)`; drive with `held`, `heldEvents` and `release`. */
export function holdReplies({ asked = [], events = false }: Hold) {
  const urls = new Map<string, string>(); // request_id → url
  const held: HeldItem[] = [];
  const hold = { on: true, held, urls };
  (window as unknown as { __hold: typeof hold }).__hold = hold;
  const matches = (msg: { type?: string; method?: string; url?: string }) =>
    msg.type === 'http_request'
    && asked.some((a) => a.method === msg.method && (msg.url ?? '').split('?')[0] === a.path);

  const send = RTCDataChannel.prototype.send;
  RTCDataChannel.prototype.send = function (this: RTCDataChannel, data: string | Blob | ArrayBuffer | ArrayBufferView) {
    if (hold.on && typeof data === 'string' && data.includes('http_request')) {
      try {
        const msg = JSON.parse(data);
        if (matches(msg) && msg.request_id != null) urls.set(String(msg.request_id), msg.url);
      } catch { /* a split payload — not one we hold */ }
    }
    return send.call(this, data as string);
  } as typeof RTCDataChannel.prototype.send;

  const prop = Object.getOwnPropertyDescriptor(RTCDataChannel.prototype, 'onmessage')!;
  Object.defineProperty(RTCDataChannel.prototype, 'onmessage', {
    configurable: true,
    get() { return prop.get!.call(this); },
    set(fn) {
      if (typeof fn !== 'function') return prop.set!.call(this, fn);
      prop.set!.call(this, function (this: RTCDataChannel, ev: MessageEvent) {
        if (events && hold.on && this.label.startsWith('sess-')) {
          const url = `channel:${this.label}`;
          urls.set(url, url);
          held.push({ url, data: String(ev.data), deliver: () => fn.call(this, ev) });
          return;
        }
        if (typeof ev.data === 'string' && urls.size > 0) {
          const id = /"request_id"\s*:\s*"?([^",}]+)/.exec(ev.data)?.[1];
          const url = id && urls.get(id);
          if (url) { held.push({ url, data: ev.data, deliver: () => fn.call(this, ev) }); return; }
        }
        return fn.call(this, ev);
      });
    },
  });
}

/** What is held, in the order it was asked: a request's session_id, else
 *  its url; a session channel's events as `channel:<label>`. */
export async function held(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const hold = (window as unknown as { __hold: { urls: Map<string, string> } }).__hold;
    return [...hold.urls.values()].map((u) => new URL(u, location.origin).searchParams.get('session_id') ?? u);
  });
}

type HeldItem = { url: string; data: string; deliver: () => void };

/** Stops holding and delivers what is held — those `first` names (as `held`
 *  lists them) in that order, then the rest; each one's messages in the
 *  order they came. A test that checks what did NOT land waits for a later
 *  definite point first (`expectTurnOver` after its next turn). */
export async function release(page: Page, first: string[] = []) {
  await page.evaluate((order) => {
    const hold = (window as unknown as { __hold: { on: boolean; urls: Map<string, string>; held: HeldItem[] } }).__hold;
    hold.on = false;
    const key = (u: string) => new URL(u, location.origin).searchParams.get('session_id') ?? u;
    for (const k of order) {
      for (const h of hold.held.filter((h) => key(h.url) === k)) h.deliver();
    }
    for (const h of hold.held.filter((h) => !order.includes(key(h.url)))) h.deliver();
    hold.held.length = 0;
    hold.urls.clear();
  }, first);
}

/** The held events' kinds, in the order they came: `kind`, and the text
 *  for a `message` (`message:<text>`). */
export async function heldEvents(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const hold = (window as unknown as { __hold: { held: HeldItem[] } }).__hold;
    return hold.held.filter((h) => h.url.startsWith('channel:')).map((h) => {
      const ev = JSON.parse(h.data);
      return ev.kind === 'message' ? `message:${ev.text}` : String(ev.kind);
    });
  });
}

/** Records what the page asks over the data channel — each `http_request`
 *  as `METHOD /path` — in `__asked`. Install with
 *  `page.addInitScript(watchAsked)`; read with `asked`. */
export function watchAsked() {
  const log: string[] = [];
  (window as unknown as { __asked: string[] }).__asked = log;
  const send = RTCDataChannel.prototype.send;
  RTCDataChannel.prototype.send = function (this: RTCDataChannel, data: string | Blob | ArrayBuffer | ArrayBufferView) {
    if (typeof data === 'string' && data.includes('"http_request"')) {
      try {
        const msg = JSON.parse(data);
        log.push(`${msg.method} ${String(msg.url).split('?')[0]}`);
      } catch { /* a split payload — not a request line */ }
    }
    return send.call(this, data as string);
  } as typeof RTCDataChannel.prototype.send;
}

/** What the page asked over the data channel so far (see `watchAsked`). */
export async function asked(page: Page): Promise<string[]> {
  return page.evaluate(() => (window as unknown as { __asked: string[] }).__asked);
}

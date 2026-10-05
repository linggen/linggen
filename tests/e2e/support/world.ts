// A test's own world: tests/world started as a child process (the same
// world-builder the Rust system tests use), stopped when the test ends.
// A failed test keeps the world's root (the engine and ling-mem logs) and
// attaches the engine log.
import { test as base, expect, type Page } from '@playwright/test';
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { createInterface } from 'node:readline';
import { readFileSync } from 'node:fs';
import path from 'node:path';

/** What the fake model answers, call by call, per member. */
export type Reply = { text: string } | { tool: string; args: Record<string, unknown> };
export type Scenario = {
  /** tests/fixtures/<fixture>; `home` when unset, `fresh` = a new install. */
  fixture?: string;
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

async function startWorld(scenario: Scenario): Promise<{ world: World; child: ChildProcessWithoutNullStreams }> {
  const bin = process.env.LINGGEN_WORLD_BIN;
  if (!bin) throw new Error('LINGGEN_WORLD_BIN unset — run through playwright (global setup builds it)');
  const child = spawn(bin, [JSON.stringify(scenario)], { stdio: ['pipe', 'pipe', 'pipe'] });
  let stderr = '';
  child.stderr.on('data', (d) => (stderr += d));
  const lines = createInterface({ input: child.stdout });
  const waiting: ((line: string) => void)[] = [];
  lines.on('line', (line) => waiting.shift()?.(line));
  const nextLine = () =>
    new Promise<string>((resolve, reject) => {
      waiting.push(resolve);
      child.once('exit', (code) => reject(new Error(`world exited (${code}):\n${stderr}`)));
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
  return { world, child };
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

/** Caps animation frames at 10 a second. Yinyue's avatar renders WebGL in
 *  software in headless Chromium at 60 fps — on a busy machine that starves
 *  the page's main thread (a fill took 28 s). Nothing a test reads depends
 *  on frame rate. */
function throttleAnimationFrames() {
  window.requestAnimationFrame = (cb) =>
    window.setTimeout(() => cb(performance.now()), 100) as unknown as number;
  window.cancelAnimationFrame = (id) => window.clearTimeout(id);
}

type Fixtures = {
  scenario: Scenario;
  world: World;
  /** Paths of the requests the page sent over plain HTTP (not the data channel). */
  httpRequests: string[];
};

export const test = base.extend<Fixtures>({
  scenario: [{}, { option: true }],

  world: async ({ scenario }, use, testInfo) => {
    const { world, child } = await startWorld(scenario);
    await use(world);
    const failed = testInfo.status !== testInfo.expectedStatus;
    if (failed) {
      await testInfo.attach('engine.log', { body: world.engineLog(), contentType: 'text/plain' });
      child.stdin.write('keep\n');
    }
    const exited = new Promise((r) => child.once('exit', r));
    child.stdin.end();
    await exited;
  },

  httpRequests: async ({}, use) => {
    await use([]);
  },

  page: async ({ page, httpRequests }, use) => {
    await page.addInitScript(watchPeerConnections);
    await page.addInitScript(throttleAnimationFrames);
    page.on('request', (req) => httpRequests.push(`${req.method()} ${new URL(req.url()).pathname}`));
    await use(page);
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

/** No chat request went over plain HTTP — it rode the data channel. */
export function expectNoHttpChat(httpRequests: string[]) {
  expect(httpRequests.filter((r) => /\/api\/chat\b/.test(r))).toEqual([]);
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

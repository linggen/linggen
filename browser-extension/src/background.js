// Bridge core — the extension side of doc/browser-bridge-spec.md.
//
// The extension is the WebSocket *client*: an MV3 service worker can't run a
// server, but it can hold a socket open (which also keeps the worker alive).
// The daemon pushes `req` frames; we dispatch to a module, run the read, and
// answer with a `res` frame. Reads are on-demand only — nothing is harvested
// in the background.

import { xModule } from "./modules/x.js";
import { controlModule } from "./modules/control.js";
import { beginOp, endOp, progressFor, setProgressSink } from "./progress.js";

// Daemon port. Defaults to the engine's 9527; overridable without a rebuild
// by setting `daemonPort` in chrome.storage.local (e.g. from the extension
// console or a future options page) so a non-default engine port still
// connects.
const DEFAULT_PORT = 9527;

async function daemonWsUrl() {
  const { daemonPort } = await chrome.storage.local.get("daemonPort");
  const port = Number.isInteger(daemonPort) ? daemonPort : DEFAULT_PORT;
  return `ws://127.0.0.1:${port}/api/bridge/socket`;
}
const EXT_VERSION = chrome.runtime.getManifest().version;
const MODULES = { x: xModule, control: controlModule };
const RECONNECT_MS = [1000, 2000, 5000, 10000, 30000];

let socket = null;
let attempt = 0;

function moduleList() {
  return Object.entries(MODULES).map(([id, m]) => ({ id, version: m.version }));
}

function send(frame) {
  if (socket && socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify(frame));
}

setProgressSink(send);

function fail(id, code, message) {
  send({ t: "res", id, ok: false, code, message });
}

// Each op opens a real background x.com tab and drives a live page load — a
// burst of these back-to-back (a single Rescan can queue several) is a clean
// automation fingerprint. Serialize to one in flight, with a randomized
// human-scale gap before each one starts.
const MIN_GAP_MS = 3000;
const MAX_GAP_MS = 10000;
let queue = Promise.resolve();
let lastActionAt = 0;

function paced(task) {
  queue = queue.then(async () => {
    const gap = MIN_GAP_MS + Math.random() * (MAX_GAP_MS - MIN_GAP_MS);
    const wait = lastActionAt ? gap - (Date.now() - lastActionAt) : 0;
    if (wait > 0) await new Promise((r) => setTimeout(r, wait));
    try {
      await task();
    } catch (e) {
      // A task that throws must not poison `queue` for later requests — it
      // would leave every subsequent paced() call permanently unresolved.
      console.error("[bridge] paced task failed", e);
    } finally {
      lastActionAt = Date.now();
    }
  });
  return queue;
}

// Control ops drive a visible tab the user is watching — human-scale gaps
// would just make the agent feel broken, and there is no hidden-tab burst to
// disguise. They still serialize (one at a time) so CDP calls don't interleave.
let controlQueue = Promise.resolve();

// Control ops waiting or running. One ahead means this one waits its turn,
// and the caller is told so rather than watching a spinner.
let controlPending = 0;

function serialized(task) {
  controlQueue = controlQueue.then(task).catch((e) => {
    console.error("[bridge] control task failed", e);
  });
  return controlQueue;
}

// `serialized` survives a task that THROWS, but not one that never settles —
// that wedges controlQueue for good, and every later call dies at its caller's
// timeout with no way back short of reloading the extension. Nothing legitimate
// runs this long: the gate's prompt gives up on the user after 120s, and the
// slowest real op (a full-length post, typed a character at a time, waiting on
// x.com to confirm) lands well under it. Past this, the op has hung — answer
// the caller and let the queue move on. A late response for an id the daemon
// has already given up on is dropped there, so releasing early is safe.
const QUEUE_DEADLINE_MS = 240_000;

function withDeadline(task, id) {
  return Promise.race([
    task(),
    new Promise((resolve) => setTimeout(() => {
      console.error(`[bridge] op ${id} passed the queue deadline — releasing the queue`);
      fail(id, "timeout", "the extension gave up on this op — it never finished");
      resolve();
    }, QUEUE_DEADLINE_MS)),
  ]);
}

async function handleRequest(frame) {
  const mod = MODULES[frame.module];
  if (!mod) return fail(frame.id, "module_unavailable", `no module '${frame.module}'`);
  const op = mod.ops[frame.op];
  if (!op) return fail(frame.id, "bad_request", `unknown op '${frame.op}'`);
  const run = async () => {
    beginOp(frame.id);
    try {
      const data = await op(frame.params || {});
      send({ t: "res", id: frame.id, ok: true, data });
    } catch (e) {
      fail(frame.id, e.code || "upstream_error", String(e.message || e));
    } finally {
      endOp(frame.id);
    }
  };
  // A module is paced as a whole, but one op can opt out — x's `post` drives a
  // visible tab in front of a user who just pressed a button, and the pacing
  // queue exists to disguise hidden-tab read bursts, which that is not.
  const unpaced = mod.unpaced ||
    (Array.isArray(mod.unpacedOps) && mod.unpacedOps.includes(frame.op));
  if (unpaced) {
    if (controlPending > 0) {
      progressFor(frame.id, "queued", "Waiting for another browser action to finish");
    }
    controlPending++;
    serialized(() => withDeadline(run, frame.id).finally(() => { controlPending--; }));
  } else paced(run);
}

const HANDLERS = {
  ready: () => setStatus("connected"),
  reject: (f) => console.warn("[bridge] daemon rejected:", f.reason),
  req: handleRequest,
};

function onMessage(event) {
  let frame;
  try { frame = JSON.parse(event.data); } catch { return; }
  const handler = HANDLERS[frame.t];
  if (handler) handler(frame);
}

function isLive() {
  return socket && (socket.readyState === WebSocket.OPEN || socket.readyState === WebSocket.CONNECTING);
}

// Chrome terminates an idle extension service worker after 30 seconds, and only
// WebSocket *messages* reset that timer — the protocol-level ping/pong the
// daemon answers is handled below the WebSocket API and doesn't count, nor does
// anything send one. This socket is silent whenever no skill is calling, so the
// worker was killed ~30s after every start and the one-minute alarm below was
// the only thing bringing it back: the bridge read as connected for roughly
// half of each minute, and a call landing in the other half got `no_bridge`.
//
// One message inside the window keeps the worker alive for as long as the
// socket is up. The daemon ignores frames it doesn't know, so this costs it
// nothing (see doc/browser-bridge-spec.md).
const PING_MS = 20_000;
let pingTimer = null;

function startPing() {
  stopPing();
  pingTimer = setInterval(() => {
    if (!socket || socket.readyState !== WebSocket.OPEN) return stopPing();
    send({ t: "ping" });
  }, PING_MS);
}

function stopPing() {
  if (pingTimer !== null) {
    clearInterval(pingTimer);
    pingTimer = null;
  }
}

async function connect() {
  if (isLive()) return;
  socket = new WebSocket(await daemonWsUrl());
  socket.onopen = () => {
    attempt = 0;
    send({ t: "hello", ext_version: EXT_VERSION, modules: moduleList() });
    startPing();
  };
  socket.onmessage = onMessage;
  socket.onerror = () => socket && socket.close();
  socket.onclose = () => {
    stopPing();
    scheduleReconnect();
  };
}

function scheduleReconnect() {
  setStatus("disconnected");
  const delay = RECONNECT_MS[Math.min(attempt, RECONNECT_MS.length - 1)];
  attempt += 1;
  setTimeout(connect, delay);
}

function setStatus(state) {
  chrome.storage.session.set({ bridgeStatus: state, modules: moduleList() }).catch(() => {});
}

// The keepalive above holds the worker up while the socket is OPEN. With the
// daemon down there is nothing to keep it up, so the worker dies and takes the
// backoff timer with it — this alarm is what revives it, which makes its period
// the real reconnect latency once Linggen comes back. 30s is the floor Chrome
// allows.
chrome.alarms.create("bridge-keepalive", { periodInMinutes: 0.5 });
chrome.alarms.onAlarm.addListener((a) => { if (a.name === "bridge-keepalive") connect(); });
chrome.runtime.onStartup.addListener(connect);
chrome.runtime.onInstalled.addListener(connect);

setStatus("disconnected");
connect();

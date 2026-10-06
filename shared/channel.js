// The data path of a skill page — served by the engine at /shared/channel.js.
//
// A Linggen page sends no data over plain HTTP. Every engine call a skill page
// makes — `/api/*`, and a POST to `/apps/<skill>/capability/*` — rides a WebRTC
// data channel, as the main UI's do (doc/webrtc-spec.md § Skill pages). Plain
// HTTP carries only the page's static files.
//
// Loading it routes the page's own `fetch` for those paths, so skill code that
// calls fetch('/api/…') works unchanged; /shared/api.js and
// /shared/chat-bridge.js load it themselves. A page whose inline script calls
// the API while it parses loads it first, as a classic script in <head>:
//   <script src="/shared/channel.js"></script>
// (it is also importable: import '/shared/channel.js').
//
// Which channel, nearest first:
//  1. Remote (linggen.dev — the `linggen-instance` meta): the connect page's
//     tunnel worker already answers every request of the page over its data
//     channel, so fetch goes as it is.
//  2. A same-origin frame above the page that holds a channel — the launcher,
//     or a skill page around its settings overlay — is asked directly.
//  3. Otherwise the page opens its own: a hidden engine frame (`/?channel=1`)
//     brings up one peer, the way the chat embed does, and carries the calls.
// A call made before the channel is open waits for it (20 s, then a
// NotConnectedError); nothing falls back to HTTP. Byte replies (audio,
// images) come as base64 and are handed back as the bytes they were.
//
// The page offers its channel to its own frames as `window.__linggenChannel`
// (`request`, `isOpen`) — the same face the engine's UI shows its frames.
(function (root) {
  'use strict';
  if (root.LinggenChannel) return; // loaded twice (classic + module): one is enough

  /** How long a call waits for the data channel (as the main UI's). */
  const CONNECT_WAIT_MS = 20_000;
  /** Where a frame above, or the page's own channel frame, keeps its channel. */
  const HOST_KEY = '__linggenChannel';
  /** The engine's UI as a bare channel: one peer, nothing rendered. */
  const FRAME_URL = '/?channel=1';
  /** Statuses a Response may not carry a body with. */
  const NO_BODY = new Set([101, 204, 205, 304]);

  /** The data channel did not open in time for a call. */
  class NotConnectedError extends Error {
    constructor(message) {
      super(message);
      this.name = 'NotConnectedError';
    }
  }

  function notConnected(what, waitedMs) {
    return new NotConnectedError(
      `Not connected to Linggen: ${what} waited ${Math.round(waitedMs / 1000)} s for the data channel`);
  }

  /** Whether a call to this path rides the channel: the API (not its
   *  signaling) and a skill's capability calls. Static files stay HTTP. */
  function ridesChannel(path, method) {
    if (path.startsWith('/api/rtc/')) return false;
    if (path.startsWith('/api/')) return true;
    return path.startsWith('/apps/') && method !== 'GET' && method !== 'HEAD';
  }

  function isRequest(input) {
    return typeof Request !== 'undefined' && input instanceof Request;
  }

  /** `{ method, path }` of a fetch that rides the channel, else null. `base`
   *  is the page's URL: relative and same-origin absolute URLs alike. */
  function routeOf(input, init, base) {
    const url = isRequest(input) ? input.url : String(input instanceof URL ? input.href : input);
    const method = String((init && init.method) || (isRequest(input) ? input.method : 'GET')).toUpperCase();
    let u;
    try {
      u = new URL(url, base);
    } catch {
      return null;
    }
    if (u.origin !== new URL(base).origin) return null;
    if (!ridesChannel(u.pathname, method)) return null;
    return { method, path: u.pathname + u.search };
  }

  /** The body as the channel carries it: text (JSON is text). */
  function textBody(raw, what) {
    if (raw == null || raw === '') return undefined;
    if (typeof raw === 'string') return raw;
    if (typeof URLSearchParams !== 'undefined' && raw instanceof URLSearchParams) return raw.toString();
    throw new TypeError(`${what}: the data channel carries text and JSON bodies only`);
  }

  function isJson(text) {
    try {
      JSON.parse(text);
      return true;
    } catch {
      return false;
    }
  }

  /** `{ body, contentType }` of a fetch: the type it names, or JSON when the
   *  text is JSON (the engine re-sends a JSON body as JSON). */
  async function bodyOf(input, init, what) {
    const own = init && init.body !== undefined;
    const raw = own ? init.body : isRequest(input) ? await input.text() : undefined;
    const body = textBody(raw, what);
    const headers = new Headers((init && init.headers) || (isRequest(input) ? input.headers : undefined));
    const named = headers.get('content-type');
    const contentType = named || (body !== undefined && isJson(body) ? 'application/json' : undefined);
    return { body, contentType };
  }

  function fromBase64(b64) {
    const binary = atob(b64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return bytes;
  }

  /** The engine's reply as the Response the page's fetch resolves to —
   *  bytes intact when they came as base64. */
  function toResponse(reply) {
    const status = reply.status || 200;
    const headers = { 'Content-Type': reply.content_type || 'application/json' };
    if (NO_BODY.has(status)) return new Response(null, { status, headers });
    const body = reply.body_encoding === 'base64'
      ? fromBase64(reply.body || '')
      : typeof reply.body === 'string' ? reply.body : JSON.stringify(reply.body ?? '');
    return new Response(body, { status, headers });
  }

  /** A host's error as one of this page's own (it may come from another frame). */
  function localError(e) {
    if (e && e.name === 'NotConnectedError') return new NotConnectedError(e.message);
    if (e && e.name === 'AbortError') return e;
    return new Error((e && e.message) || String(e));
  }

  /** The nearest same-origin frame above `win` that holds a channel. A frame
   *  of another origin ends the search: its channel is not this page's. */
  function ancestorHost(win) {
    let w = win;
    while (w.parent && w.parent !== w) {
      w = w.parent;
      let host;
      try {
        host = w[HOST_KEY];
      } catch {
        return null;
      }
      if (host) return host;
    }
    return null;
  }

  /** Opened through linggen.dev: its tunnel worker carries every request. */
  function isRemote(doc) {
    return !!doc.querySelector('meta[name="linggen-instance"]');
  }

  /** The page's own channel: a hidden engine frame, which opens one peer.
   *  Kept outside <body>, so a page that rewrites its body keeps it. */
  function openFrame(doc) {
    return new Promise((resolve, reject) => {
      const frame = doc.createElement('iframe');
      frame.src = FRAME_URL;
      frame.title = 'Linggen channel';
      frame.tabIndex = -1;
      frame.setAttribute('aria-hidden', 'true');
      frame.style.display = 'none';
      frame.addEventListener('load', () => {
        let host = null;
        try {
          host = frame.contentWindow[HOST_KEY] || null;
        } catch { /* not the engine's page */ }
        if (host) resolve({ host, alive: () => frame.isConnected });
        else reject(new NotConnectedError('Not connected to Linggen: its channel page did not load'));
      }, { once: true });
      doc.documentElement.appendChild(frame);
    });
  }

  function toBase64(buf) {
    const bytes = new Uint8Array(buf);
    let binary = '';
    for (let i = 0; i < bytes.length; i += 0x8000) {
      binary += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
    }
    return btoa(binary);
  }

  /** One `request` on a remote page: its fetch, which the tunnel worker
   *  answers over the connect page's data channel — as the engine's reply. */
  async function viaTunnel(fetchFn, req) {
    const headers = req.contentType ? { 'Content-Type': req.contentType } : undefined;
    const res = await fetchFn(req.path, { method: req.method, headers, body: req.body });
    const contentType = res.headers.get('content-type') || '';
    if (/^(text\/|application\/(json|javascript)|image\/svg)/.test(contentType) || !contentType) {
      return { status: res.status, content_type: contentType, body: await res.text() };
    }
    const body = toBase64(await res.arrayBuffer());
    return { status: res.status, content_type: contentType, body, body_encoding: 'base64' };
  }

  /** Settles with `promise`, or rejects with NotConnectedError after `ms`. */
  function within(promise, ms, what) {
    let timer;
    const late = new Promise((_, reject) => {
      timer = setTimeout(() => reject(notConnected(what, ms)), ms);
    });
    return Promise.race([promise, late]).finally(() => clearTimeout(timer));
  }

  /** Rejects as fetch does once `signal` aborts. */
  function abortable(promise, signal) {
    if (!signal) return promise;
    const reason = () => signal.reason || new DOMException('The operation was aborted.', 'AbortError');
    if (signal.aborted) return Promise.reject(reason());
    return new Promise((resolve, reject) => {
      signal.addEventListener('abort', () => reject(reason()), { once: true });
      promise.then(resolve, reject);
    });
  }

  /**
   * One page's channel. `env`: `win`, `doc`, `fetch` (the browser's own),
   * `openFrame()` → Promise<{ host, alive() }>, `waitMs`. Tests pass fakes.
   */
  function createChannel(env) {
    const waitMs = env.waitMs ?? CONNECT_WAIT_MS;
    const remote = isRemote(env.doc);
    let own = null; // Promise<{ host, alive }> of the page's channel frame
    let ownReady = null; // that frame, once it is up

    function openOwn() {
      if (ownReady && !ownReady.alive()) own = ownReady = null; // removed: open again
      if (!own) {
        own = env.openFrame().then(
          (f) => (ownReady = f),
          (e) => {
            own = null;
            throw e;
          },
        );
        own.catch(() => {}); // a caller that stopped waiting still hears it
      }
      return own;
    }

    /** The host to ask now, if there is one: a frame above, or our own. */
    function hostNow() {
      return ancestorHost(env.win) || (ownReady && ownReady.alive() ? ownReady.host : null);
    }

    /** The host to ask, waiting for the page's own to come up when `wait`. */
    async function hostFor(what, wait) {
      const now = hostNow();
      if (now) return now;
      const opening = openOwn();
      if (!wait) throw notConnected(what, 0);
      return (await within(opening, waitMs, what)).host;
    }

    /** One call over the channel: `{ method, path, body?, contentType?,
     *  wait? }` → the engine's reply `{ status, content_type, body,
     *  body_encoding? }`. `wait: false` fails at once when it is not open
     *  (a reading, like the presence beat, is skipped, never queued). */
    async function request(req) {
      if (remote) return viaTunnel(env.fetch, req);
      const what = `${req.method} ${req.path}`;
      const host = await hostFor(what, req.wait !== false);
      try {
        const r = await host.request(req);
        return { status: r.status, content_type: r.content_type, body: r.body, body_encoding: r.body_encoding };
      } catch (e) {
        throw localError(e);
      }
    }

    async function send(route, input, init) {
      const what = `${route.method} ${route.path}`;
      const { body, contentType } = await bodyOf(input, init, what);
      const reply = await request({ method: route.method, path: route.path, body, contentType });
      return toResponse(reply);
    }

    /** `fetch`, with engine calls over the channel; anything else as is. */
    function channelFetch(input, init) {
      const route = remote ? null : routeOf(input, init, env.win.location.href);
      if (!route) return env.fetch(input, init);
      return abortable(send(route, input, init), init && init.signal);
    }

    function isOpen() {
      if (remote) return true;
      const host = hostNow();
      return !!host && !!host.isOpen();
    }

    return { fetch: channelFetch, request, isOpen, remote };
  }

  /** Route this page's `fetch` and offer its channel to its own frames. */
  function install(win, doc) {
    const native = win.fetch.bind(win);
    const channel = createChannel({ win, doc, fetch: native, openFrame: () => openFrame(doc) });
    if (!channel.remote) {
      win.fetch = channel.fetch;
      win[HOST_KEY] = { request: channel.request, isOpen: channel.isOpen };
    }
    return channel;
  }

  const api = {
    CONNECT_WAIT_MS,
    NotConnectedError,
    ridesChannel,
    routeOf,
    bodyOf,
    toResponse,
    ancestorHost,
    createChannel,
  };
  const inPage = typeof document !== 'undefined' && typeof root.fetch === 'function';
  if (inPage) Object.assign(api, install(root, document));
  root.LinggenChannel = api;
})(typeof window !== 'undefined' ? window : globalThis);

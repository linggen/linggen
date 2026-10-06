/**
 * The UI's one data path: every `/api/*` call rides the WebRTC data channel.
 * Plain HTTP carries only the page, its static files and the `/api/rtc/*`
 * signaling that brings the channel up (doc/webrtc-spec.md).
 *
 * A call made before the channel is open waits for it (`ConnectionGate`) —
 * at boot, and while a dropped link reconnects — and fails with
 * `NotConnectedError` when it does not open in time. It never falls back to
 * HTTP: a failed call is an error. Pure, so node tests can hold it;
 * fetchProxy.ts wires it to `window.fetch` and the transport.
 */

/** How long a call waits for the data channel. Boot fits well inside it
 *  (ICE gathering is capped at 5 s, then one signaling round trip, or the
 *  relay's polling remotely); a link down longer than this fails the call. */
export const CONNECT_WAIT_MS = 20_000;

/** The data channel did not open in time for a call. */
export class NotConnectedError extends Error {
  constructor(what: string, waitedMs: number) {
    super(`Not connected to Linggen: ${what} waited ${Math.round(waitedMs / 1000)} s for the data channel`);
    this.name = 'NotConnectedError';
  }
}

/** Open while the data channel is; calls wait on it while it is not. */
export interface ConnectionGate {
  isOpen(): boolean;
  /** The transport's status changed: open when connected, closed otherwise. */
  set(open: boolean): void;
  /** Settles once the gate is open — at once if it is — or rejects with
   *  `NotConnectedError` after `waitMs`. `what` names the call in the error. */
  whenOpen(what: string, waitMs?: number): Promise<void>;
}

export function createGate(): ConnectionGate {
  let open = false;
  const waiters = new Set<() => void>();
  return {
    isOpen: () => open,
    set(next: boolean) {
      open = next;
      if (!open) return;
      for (const release of [...waiters]) release();
      waiters.clear();
    },
    whenOpen(what: string, waitMs = CONNECT_WAIT_MS) {
      if (open) return Promise.resolve();
      return new Promise<void>((resolve, reject) => {
        const release = () => {
          clearTimeout(timer);
          resolve();
        };
        const timer = setTimeout(() => {
          waiters.delete(release);
          reject(new NotConnectedError(what, waitMs));
        }, waitMs);
        waiters.add(release);
      });
    },
  };
}

/** Paths whose fetch rides the data channel: the API (but its signaling,
 *  which must reach the engine before there is a channel) and what a remote
 *  page can only load through it — lazy chunks, skill pages, the logo. */
export function ridesChannel(path: string): boolean {
  if (path.startsWith('/api/rtc/')) return false;
  return path.startsWith('/api/')
    || path.startsWith('/assets/')
    || path.startsWith('/apps/')
    || path === '/logo.svg';
}

/** A same-origin absolute URL as its path (`https://host/api/x?y` →
 *  `/api/x?y`), so it routes like the relative form; other URLs as given. */
export function toPath(url: string, origin: string): string {
  return url.startsWith(origin + '/') ? url.slice(origin.length) : url;
}

/** The request's body as the channel carries it: JSON bodies parsed (the
 *  engine re-sends them as JSON), anything else as given. */
export function channelBody(body: unknown, contentType: string | null | undefined): unknown {
  if (body == null || body === '') return undefined;
  if (typeof body !== 'string' || !contentType?.includes('application/json')) return body;
  try {
    return JSON.parse(body);
  } catch {
    return body;
  }
}

/** The engine's answer to an `http_request`: the loopback reply's status,
 *  content type and body — base64 when the body is bytes (audio, images). */
export type ProxyReply = {
  status: number;
  body: string;
  content_type?: string;
  body_encoding?: string;
};

/** Statuses a Response may not carry a body with. */
const NO_BODY = new Set([101, 204, 205, 304]);

function fromBase64(b64: string): Uint8Array {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

/** The engine's reply as the `Response` the caller's `fetch` resolves to —
 *  bytes intact when they came as base64. */
export function toResponse(reply: ProxyReply): Response {
  const status = reply.status || 200;
  const headers = { 'Content-Type': reply.content_type || 'application/json' };
  if (NO_BODY.has(status)) return new Response(null, { status, headers });
  const body = reply.body_encoding === 'base64'
    ? fromBase64(reply.body ?? '')
    : typeof reply.body === 'string' ? reply.body : JSON.stringify(reply.body ?? '');
  return new Response(body as BodyInit, { status, headers });
}

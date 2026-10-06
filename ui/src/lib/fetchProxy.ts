/**
 * Routes the page's `fetch` over the WebRTC data channel.
 *
 * Every `/api/*` call (and, for a remote page, its lazy chunks and skill
 * files) goes out as an `http_request` on the control channel — locally and
 * remotely alike. A call made before the channel is open waits for it and
 * fails with a clear error if it does not open in time; nothing falls back to
 * plain HTTP (channelFetch.mts). Signaling (`/api/rtc/*`) and static files
 * use `_originalFetch`.
 *
 * Call `installFetchProxy()` once at app startup to activate.
 */
import { connection, getTransport } from './transport';
import { channelBody, ridesChannel, toPath, toResponse } from './channelFetch.mts';

/** The browser's own fetch — plain HTTP. Only for what brings the channel
 *  up (signaling) and the page's static files; never for data. */
export const _originalFetch = window.fetch.bind(window);

function urlOf(input: RequestInfo | URL): string {
  if (typeof input === 'string') return input;
  return input instanceof URL ? input.toString() : input.url;
}

function methodOf(input: RequestInfo | URL, init?: RequestInit): string {
  const method = init?.method ?? (input instanceof Request ? input.method : 'GET');
  return method.toUpperCase();
}

function contentTypeOf(headers: HeadersInit | undefined): string | null {
  if (!headers) return null;
  return new Headers(headers).get('content-type');
}

/** One call over the data channel, waiting for it to open first. */
async function channelFetch(path: string, method: string, init?: RequestInit): Promise<Response> {
  await connection.whenOpen(`${method} ${path}`);
  const body = channelBody(init?.body, contentTypeOf(init?.headers));
  const reply = await getTransport().httpProxy(method, path, body);
  return toResponse(reply);
}

/** Route `window.fetch`: channel paths over the data channel, the rest as is. */
export function installFetchProxy(): void {
  window.fetch = ((input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const path = toPath(urlOf(input), window.location.origin);
    if (!ridesChannel(path)) return _originalFetch(input, init);
    return channelFetch(path, methodOf(input, init), init);
  }) as typeof window.fetch;
}

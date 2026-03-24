/**
 * Transport-aware fetch proxy.
 *
 * When using WebRTC transport, all `/api/*` requests are routed through
 * the WebRTC control data channel instead of direct HTTP. This makes
 * every existing fetch() call work in remote mode without changes.
 *
 * For SSE transport (or non-API requests), the original fetch is used.
 *
 * Call `installFetchProxy()` once at app startup to activate.
 */
import { getTransport } from './transport';
import { RtcTransport } from './rtcTransport';

/** The original window.fetch — use this for direct HTTP access. */
export const _originalFetch = window.fetch.bind(window);

/** Check if the current transport is WebRTC. */
function isWebRtcTransport(): boolean {
  try {
    return getTransport() instanceof RtcTransport;
  } catch {
    return false;
  }
}

/**
 * Proxy a fetch request through the WebRTC control channel.
 * Serializes the request, sends it as an http_request message,
 * and constructs a Response from the reply.
 */
async function rtcFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url;
  const method = init?.method || 'GET';
  const contentType = init?.headers instanceof Headers
    ? init.headers.get('content-type')
    : Array.isArray(init?.headers)
      ? init.headers.find(([k]) => k.toLowerCase() === 'content-type')?.[1]
      : (init?.headers as Record<string, string>)?.['Content-Type'];

  let body: any = undefined;
  if (init?.body) {
    if (typeof init.body === 'string') {
      // Try to parse as JSON for structured transport
      if (contentType?.includes('application/json')) {
        try { body = JSON.parse(init.body); } catch { body = init.body; }
      } else {
        body = init.body;
      }
    } else {
      body = init.body;
    }
  }

  const transport = getTransport();
  const result = await transport.httpProxy(method, url, body);

  // Construct a Response from the proxy result
  const status = result.status || 200;
  const responseBody = typeof result.body === 'string' ? result.body : JSON.stringify(result.body || '');

  // Infer content type from URL for non-API requests
  const contentTypeHeader = url.endsWith('.js') ? 'application/javascript'
    : url.endsWith('.css') ? 'text/css'
    : url.endsWith('.html') ? 'text/html'
    : 'application/json';

  return new Response(responseBody, {
    status,
    headers: { 'Content-Type': contentTypeHeader },
  });
}

/**
 * Install the transport-aware fetch proxy.
 * After calling this, all `fetch('/api/...')` calls will be routed
 * through WebRTC when using RtcTransport.
 */
export function installFetchProxy(): void {
  console.log('[fetchProxy] installed');
  window.fetch = ((input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url;

    // Proxy /api/* and /assets/* requests through WebRTC when connected.
    // /api/rtc/* is excluded — it must use direct HTTP for signaling.
    // /assets/* is proxied only for remote/tunnel mode (lazy-loaded JS/CSS chunks).
    const shouldProxy = typeof url === 'string' && (
      (url.startsWith('/api/') && !url.includes('/api/rtc/'))
      || url.startsWith('/assets/')
      || url.startsWith('/apps/')
      || url === '/logo.svg'
    );

    if (shouldProxy && url.includes('/api/')) {
      const isRtc = isWebRtcTransport();
      const connected = isRtc ? (() => { try { return getTransport().status() === 'connected'; } catch { return false; } })() : false;
      const isRemote = !!document.querySelector('meta[name="linggen-instance"]');
      console.log(`[fetchProxy] ${url} shouldProxy=${shouldProxy} isRtc=${isRtc} connected=${connected} isRemote=${isRemote}`);
    }

    if (isWebRtcTransport() && shouldProxy) {
      try {
        const transport = getTransport();
        if (transport.status() === 'connected') {
          // Try WebRTC proxy, fall back to direct HTTP on failure
          return rtcFetch(input, init).catch((err) => {
            console.warn('WebRTC fetch proxy failed, falling back to HTTP:', err.message);
            return _originalFetch(input, init);
          });
        }
      } catch {
        // Transport not ready
      }
    }

    // In remote mode, API calls must go through WebRTC — not directly to linggen.dev.
    // Before transport is connected, return safe empty responses to prevent crashes.
    const isRemote = !!document.querySelector('meta[name="linggen-instance"]');
    if (shouldProxy && isRemote && typeof url === 'string' && url.startsWith('/')) {
      const emptyBody =
        url.includes('/sessions') ? '{"sessions":[]}' :
        url.includes('/projects') ? '[]' :
        url.includes('/skills') ? '[]' :
        url.includes('/models') ? '[]' :
        url.includes('/agents') ? '[]' :
        '{}';
      return Promise.resolve(new Response(emptyBody, { status: 200, headers: { 'Content-Type': 'application/json' } }));
    }

    return _originalFetch(input, init);
  }) as typeof window.fetch;
}

/** Restore the original fetch (for testing/cleanup). */
export function uninstallFetchProxy(): void {
  window.fetch = _originalFetch;
}

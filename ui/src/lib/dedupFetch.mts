/**
 * Deduplicated fetch — prevents the same GET request from being sent
 * multiple times concurrently. If a request for the same URL is already
 * in flight, the existing promise is reused instead of firing a new one.
 *
 * Only applies to GET requests (the main source of re-fetch storms).
 * POST/PUT/DELETE are always sent immediately.
 *
 * Implementation: stores the response body as an ArrayBuffer so it can
 * be shared across multiple callers (Response.clone() fails if the body
 * was already consumed by the first caller).
 */

interface CachedResponse {
  status: number;
  statusText: string;
  headers: [string, string][];
  body: ArrayBuffer;
}

const inflight = new Map<string, Promise<CachedResponse>>();

function toResponse(cached: CachedResponse): Response {
  return new Response(cached.body.slice(0), {
    status: cached.status,
    statusText: cached.statusText,
    headers: cached.headers,
  });
}

/**
 * Fetch with deduplication. Same-URL GET requests that are already
 * in flight will share the same promise instead of creating a new request.
 * A revision separates loads across a local history reset.
 */
export function dedupFetch(input: RequestInfo | URL, init?: RequestInit, revision = 0): Promise<Response> {
  const method = init?.method?.toUpperCase() || 'GET';
  if (method !== 'GET') return fetch(input, init);

  const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url;

  const key = `${revision}:${url}`;
  const existing = inflight.get(key);
  if (existing) return existing.then(toResponse);

  const promise = fetch(input, init).then(async (resp): Promise<CachedResponse> => ({
    status: resp.status,
    statusText: resp.statusText,
    headers: [...resp.headers.entries()],
    body: await resp.arrayBuffer(),
  }));

  inflight.set(key, promise);
  // Settled either way — the fetch or its body read failed too — it is no
  // longer in flight: the next load asks again.
  promise.then(
    () => { if (inflight.get(key) === promise) inflight.delete(key); },
    () => { if (inflight.get(key) === promise) inflight.delete(key); },
  );
  return promise.then(toResponse);
}

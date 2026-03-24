/**
 * Deduplicated fetch — prevents the same GET request from being sent
 * multiple times concurrently. If a request for the same URL is already
 * in flight, the existing promise is reused instead of firing a new one.
 *
 * Only applies to GET requests (the main source of re-fetch storms).
 * POST/PUT/DELETE are always sent immediately.
 */

const inflight = new Map<string, Promise<Response>>();

/**
 * Fetch with deduplication. Same-URL GET requests that are already
 * in flight will share the same promise instead of creating a new request.
 */
export function dedupFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const method = init?.method?.toUpperCase() || 'GET';
  if (method !== 'GET') return fetch(input, init);

  const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url;

  const existing = inflight.get(url);
  if (existing) return existing.then(r => r.clone());

  const promise = fetch(input, init).then(
    (resp) => { inflight.delete(url); return resp; },
    (err) => { inflight.delete(url); throw err; },
  );

  inflight.set(url, promise);
  return promise;
}

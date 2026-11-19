// node --test ui/tests/*.test.mts (Node strips the types).
// A failed load is never kept for the next caller. Found by the whole-code
// review (2026-11-19): when the response came but reading its body failed,
// the failed promise stayed in the in-flight map and every later load of
// that URL got the same failure.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { dedupFetch } from '../src/lib/dedupFetch.mts';

test('a body read that failed is tried again on the next load', async () => {
  let calls = 0;
  const realFetch = globalThis.fetch;
  globalThis.fetch = (async () => {
    calls++;
    if (calls === 1) {
      // The response arrives; its body cannot be read.
      return { status: 200, statusText: 'OK', headers: new Headers(), arrayBuffer: () => Promise.reject(new Error('reset')) } as unknown as Response;
    }
    return new Response('fine');
  }) as typeof fetch;
  try {
    await assert.rejects(dedupFetch('/api/x', undefined, 7));
    const second = await dedupFetch('/api/x', undefined, 7);
    assert.equal(await second.text(), 'fine');
    assert.equal(calls, 2);
  } finally {
    globalThis.fetch = realFetch;
  }
});

test('two loads at once share one request', async () => {
  let calls = 0;
  const realFetch = globalThis.fetch;
  globalThis.fetch = (async () => { calls++; return new Response('one'); }) as typeof fetch;
  try {
    const [a, b] = await Promise.all([dedupFetch('/api/y', undefined, 8), dedupFetch('/api/y', undefined, 8)]);
    assert.equal(await a.text(), 'one');
    assert.equal(await b.text(), 'one');
    assert.equal(calls, 1);
  } finally {
    globalThis.fetch = realFetch;
  }
});

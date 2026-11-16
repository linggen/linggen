import { test, mock, afterEach } from 'node:test';
import assert from 'node:assert/strict';
import { draftOf, isChanged, parseDraft, readSettings, saveSettings } from '../src/components/memory/memoryConfigApi.mts';
import { MemoryError } from '../src/components/memory/memoryApi.mts';

afterEach(() => mock.restoreAll());

const stored = { episodic_ttl_days: 7, recall_min_score: 0.7 };

test('reading asks the engine for the two settings', async () => {
  const sent: { path: string; method: string | undefined; body: unknown }[] = [];
  mock.method(globalThis, 'fetch', async (path: string, init: RequestInit) => {
    sent.push({ path, method: init.method, body: init.body });
    return new Response(JSON.stringify({ ok: true, data: stored }));
  });
  assert.deepEqual(await readSettings(), stored);
  assert.deepEqual(sent, [{ path: '/api/ling-mem/config', method: 'GET', body: undefined }]);
});

test('saving puts exactly the two fields as JSON', async () => {
  const sent: { path: string; method: string | undefined; type: unknown; body: unknown }[] = [];
  mock.method(globalThis, 'fetch', async (path: string, init: RequestInit) => {
    sent.push({ path, method: init.method, type: (init.headers as Record<string, string>)['Content-Type'], body: JSON.parse(String(init.body)) });
    return new Response(JSON.stringify({ ok: true, data: { episodic_ttl_days: 30, recall_min_score: 0.5 } }));
  });
  const back = await saveSettings({ episodic_ttl_days: 30, recall_min_score: 0.5 });
  assert.deepEqual(back, { episodic_ttl_days: 30, recall_min_score: 0.5 });
  assert.deepEqual(sent, [{
    path: '/api/ling-mem/config', method: 'PUT', type: 'application/json',
    body: { episodic_ttl_days: 30, recall_min_score: 0.5 },
  }]);
});

test('a refusal keeps its status, so the owner-only line can be told apart', async () => {
  mock.method(globalThis, 'fetch', async () => new Response(
    JSON.stringify({ ok: false, error: "Only this Mac's owner can change these.", code: 'FORBIDDEN' }), { status: 403 }));
  await assert.rejects(readSettings(), (e: unknown) => e instanceof MemoryError && e.forbidden && /owner/.test(e.message));
});

test('an answer that is not an envelope is an error with the status', async () => {
  mock.method(globalThis, 'fetch', async () => new Response('oops', { status: 500 }));
  await assert.rejects(saveSettings(stored), (e: unknown) => e instanceof MemoryError && e.status === 500);
});

test('typed values become settings, or one line saying what is wrong', () => {
  assert.deepEqual(parseDraft({ days: ' 30 ', score: '0.55' }), { settings: { episodic_ttl_days: 30, recall_min_score: 0.55 } });
  assert.deepEqual(parseDraft({ days: '3650', score: '0' }), { settings: { episodic_ttl_days: 3650, recall_min_score: 0 } });
  for (const days of ['', '0', '3651', '7.5', '-1', 'abc']) {
    assert.match((parseDraft({ days, score: '0.7' }) as { error: string }).error, /Days/, days);
  }
  for (const score of ['', '1.1', '-0.1', 'high']) {
    assert.match((parseDraft({ days: '7', score }) as { error: string }).error, /threshold/, score);
  }
});

test('Save is for a change only', () => {
  assert.equal(isChanged(draftOf(stored), stored), false);
  assert.equal(isChanged({ days: '7 ', score: '0.7' }, stored), false);
  assert.equal(isChanged({ days: '8', score: '0.7' }, stored), true);
  assert.equal(isChanged({ days: '7', score: '0.75' }, stored), true);
});

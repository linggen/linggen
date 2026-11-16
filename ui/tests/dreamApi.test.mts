import { test, mock, afterEach } from 'node:test';
import assert from 'node:assert/strict';
import { startDream } from '../src/components/memory/dreamApi.mts';

afterEach(() => mock.restoreAll());

test('dream starts all days or one attended day', async () => {
  const sent: { path: string; method: string | undefined; body: unknown }[] = [];
  mock.method(globalThis, 'fetch', async (path: string, init: RequestInit) => {
    sent.push({ path, method: init.method, body: JSON.parse(String(init.body)) });
    return new Response('{}');
  });
  await startDream();
  await startDream('2026-01-02');
  assert.deepEqual(sent, [
    { path: '/api/missions/dream/trigger', method: 'POST', body: {} },
    { path: '/api/missions/dream/trigger', method: 'POST', body: { day: '2026-01-02', attended: true } },
  ]);
});

test('a conflict says a dream is running', async () => {
  mock.method(globalThis, 'fetch', async () => new Response('', { status: 409 }));
  await assert.rejects(startDream(), /A dream is already running\./);
});

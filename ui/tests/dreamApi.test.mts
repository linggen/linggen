import { test, mock, afterEach } from 'node:test';
import assert from 'node:assert/strict';
import { startDream, startScan, startSolve } from '../src/components/memory/dreamApi.mts';

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

test('scan and solve send attended requests and return session ids', async () => {
  const sent: { path: string; method: string | undefined; body: unknown }[] = [];
  mock.method(globalThis, 'fetch', async (path: string, init: RequestInit) => {
    sent.push({ path, method: init.method, body: JSON.parse(String(init.body)) });
    return Response.json({ session_id: 'run-1' });
  });
  assert.equal(await startScan('2026-01-02'), 'run-1');
  assert.equal(await startSolve(), 'run-1');
  assert.equal(await startDream(), 'run-1');
  assert.deepEqual(sent.slice(0, 2), [
    { path: '/api/missions/memory-scan/trigger', method: 'POST', body: { day: '2026-01-02', attended: true } },
    { path: '/api/missions/memory-solve/trigger', method: 'POST', body: { attended: true } },
  ]);
});

for (const [name, start, conflict] of [
  ['scan', () => startScan('2026-01-02'), 'A scan is already running.'],
  ['solve', startSolve, 'Solve is already running.'],
] as const) {
  test(`${name} reports conflict and missing mission`, async () => {
    const fetch = mock.method(globalThis, 'fetch', async () => new Response('', { status: 409 }));
    await assert.rejects(start(), { message: conflict });
    fetch.mock.mockImplementation(async () => new Response('', { status: 404 }));
    await assert.rejects(start(), { message: 'This needs a newer Linggen.' });
  });
}

test('a conflict says a dream is running', async () => {
  mock.method(globalThis, 'fetch', async () => new Response('', { status: 409 }));
  await assert.rejects(startDream(), /A dream is already running\./);
});

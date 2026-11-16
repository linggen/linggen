// node --test ui/tests/*.test.mts
// The Memory tab's door: every call is `POST /api/memory/<verb>` with a JSON
// body; the embedding vector never gets past it.
import { test, mock, afterEach } from 'node:test';
import assert from 'node:assert/strict';
import {
  MemoryError, addBody, describeError, isNarrowed, listBody, memory, searchBody, tabScope, updateBody, whereBody, withoutVectors,
} from '../src/components/memory/memoryApi.mts';

type Sent = { path: string; body: Record<string, unknown> };

/** Replace `fetch` with a door that answers each call with `reply`. */
function door(reply: (verb: string) => { status?: number; envelope: unknown }) {
  const sent: Sent[] = [];
  mock.method(globalThis, 'fetch', async (path: string, init: RequestInit) => {
    sent.push({ path, body: JSON.parse(String(init.body)) });
    const { status = 200, envelope } = reply(path.split('/').pop()!);
    return new Response(JSON.stringify(envelope), { status, headers: { 'content-type': 'application/json' } });
  });
  return sent;
}

afterEach(() => mock.restoreAll());

const vector = Array.from({ length: 4 }, (_, i) => i / 10);
const alex = { id: 'a1', content: 'Alex lives in Lisbon.', type: 'fact', tier: 'core', from: 'user', vector };

test('a tab names its table and tier; All names neither', () => {
  assert.deepEqual(tabScope('all'), {});
  assert.deepEqual(tabScope('core'), { tier: 'core', episodic: false });
  assert.deepEqual(tabScope('semantic'), { tier: 'semantic', episodic: false });
  assert.deepEqual(tabScope('episodic'), { episodic: true });
});

test('a list asks one page at an offset, with the tab, filters and sort', () => {
  assert.deepEqual(listBody('semantic', { type: 'preference', from: 'user' }, 'oldest', 50), {
    tier: 'semantic', episodic: false, type: 'preference', from: 'user', sort: 'oldest', limit: 50, offset: 50,
  });
  assert.deepEqual(listBody('all', {}, 'newest', 0), { sort: 'newest', limit: 50, offset: 0 });
});

test('a search lifts the recall floor and has no offset', () => {
  assert.deepEqual(searchBody('where does Alex live', 'episodic', { type: 'fact' }), {
    query: 'where does Alex live', episodic: true, type: 'fact', limit: 50, min_score: 0,
  });
});

test('an add always names its tier, and a row about the person is global', () => {
  assert.deepEqual(addBody({ content: 'Alex likes tea.', tier: 'core', type: 'fact', from: 'user', scope: '~/x', indexed: true }), {
    content: 'Alex likes tea.', type: 'fact', from: 'user', tier: 'core', global: true,
  });
  assert.deepEqual(addBody({ content: 'Ran the tests.', tier: 'episodic', type: 'built', from: 'user' }), {
    content: 'Ran the tests.', type: 'built', from: 'user', episodic: true, global: true,
  });
  assert.deepEqual(addBody({ content: 'Tabs here.', tier: 'semantic', type: 'preference', from: 'user', scope: '~/harbor', indexed: true, summary: 'Tabs', outcome: 'positive' }), {
    content: 'Tabs here.', type: 'preference', from: 'user', tier: 'semantic', indexed: true, summary: 'Tabs', outcome: 'positive', scope: '~/harbor',
  });
});

test('an update is directed by the person, and clears a field with its clear_ twin', () => {
  assert.deepEqual(updateBody('a1', { content: 'New', tier: 'core', summary: null, scope: '~/x', outcome: null }), {
    id: 'a1', user_directed: true, content: 'New', tier: 'core', clear_summary: true, scope: '~/x', clear_outcome: true,
  });
  assert.deepEqual(updateBody('a1', { indexed: false }), { id: 'a1', user_directed: true, indexed: false });
});

test('count and forget address one table at a time, with the same filter', () => {
  assert.deepEqual(whereBody('core', { type: 'fact', until: '2026-01-01' }), {
    tier: 'core', episodic: false, type: 'fact', until: '2026-01-01',
  });
  assert.equal(isNarrowed({}), false);
  assert.equal(isNarrowed({ from: 'agent' }), true);
});

test('vectors are removed at any depth', () => {
  const { vector: _gone, ...bare } = alex;
  assert.deepEqual(withoutVectors({ action: 'added', fact: alex }), { action: 'added', fact: bare });
  assert.deepEqual(withoutVectors([alex]), [bare]);
});

test('list posts to the door and returns rows without their vector', async () => {
  const sent = door(() => ({ envelope: { ok: true, data: [alex] } }));
  const rows = await memory.list('core', {}, 'newest', 0);
  assert.equal(sent.length, 1);
  assert.equal(sent[0].path, '/api/memory/list');
  assert.deepEqual(sent[0].body, { tier: 'core', episodic: false, sort: 'newest', limit: 50, offset: 0 });
  assert.equal(rows[0].content, 'Alex lives in Lisbon.');
  assert.equal('vector' in rows[0], false);
});

test('add unwraps the fact, strips its vector and reports a merge', async () => {
  door(() => ({ envelope: { ok: true, data: { action: 'merged', previous_id: 'a1', fact: alex } } }));
  const out = await memory.add({ content: alex.content, tier: 'core', type: 'fact', from: 'user' });
  assert.equal(out.merged, true);
  assert.equal(out.row.id, 'a1');
  assert.equal('vector' in out.row, false);
});

test('count over every tier asks each table on its own and sums', async () => {
  const sent = door(() => ({ envelope: { ok: true, data: { count: 2 } } }));
  assert.equal(await memory.count('all', { type: 'tried' }), 6);
  assert.deepEqual(sent.map((s) => s.path), Array(3).fill('/api/memory/count'));
  assert.deepEqual(sent.map((s) => s.body), [
    { tier: 'core', episodic: false, type: 'tried' },
    { tier: 'semantic', episodic: false, type: 'tried' },
    { episodic: true, type: 'tried' },
  ]);
});

test('forget refuses a filter the person did not choose, before any call', async () => {
  const sent = door(() => ({ envelope: { ok: true, data: { removed: 0 } } }));
  await assert.rejects(() => memory.forget('core', {}), MemoryError);
  assert.equal(sent.length, 0);
  door(() => ({ envelope: { ok: true, data: { removed: 3 } } }));
  assert.equal(await memory.forget('episodic', { until: '2026-01-01' }), 3);
});

test('remove says whether a row was there', async () => {
  door(() => ({ envelope: { ok: true, data: { id: 'x', removed: false } } }));
  assert.equal(await memory.remove('x'), false);
});

test('a refusal keeps its status, and a 403 reads as a calm line', async () => {
  door(() => ({ status: 403, envelope: { ok: false, error: 'memory does not know this caller', code: 'FORBIDDEN' } }));
  const err = await memory.list('all', {}, 'newest', 0).catch((e) => e);
  assert.ok(err instanceof MemoryError);
  assert.equal(err.forbidden, true);
  assert.equal(describeError(err, 'read memory'), 'This device is not allowed to open memory.');
});

test('other failures say what was being done', () => {
  assert.equal(describeError(new MemoryError('boom', 500, 'INTERNAL'), 'save'), 'Could not save: boom');
  assert.equal(describeError(new MemoryError('x', 502, 'UNREACHABLE'), 'read memory'), 'Memory is not reachable right now.');
});

test('review and days use the expected memory requests', async () => {
  const sent = door(() => ({ envelope: { ok: true, data: { id: 'a1', vector } } }));
  await memory.issues();
  const row = await memory.get('a1');
  await memory.dismiss('issue1');
  await memory.days();
  assert.deepEqual(sent, [
    { path: '/api/memory/issues', body: { status: 'open', limit: 1000 } },
    { path: '/api/memory/get', body: { id: 'a1' } },
    { path: '/api/memory/issue_resolve', body: { id: 'issue1', outcome: 'dismissed', note: 'dismissed from the browse page' } },
    { path: '/api/memory/days', body: {} },
  ]);
  assert.ok(row && !('vector' in row));
});

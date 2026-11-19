// node --test ui/tests/*.test.mts (Node strips the types).
// A batch delete takes off the list only what the engine deleted. Found by
// the whole-code review (2026-11-19): every selected row went, a refused
// one too, until the next list push brought it back.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { removedIds } from '../src/lib/removedIds.mts';

test('a refused delete keeps its row', async () => {
  const results = await Promise.allSettled([Promise.resolve(), Promise.reject(new Error('busy'))]);
  assert.deepEqual([...removedIds(['sess-a', 'sess-b'], results)], ['sess-a']);
});

test('every delete done: every row goes', async () => {
  const results = await Promise.allSettled([Promise.resolve(), Promise.resolve()]);
  assert.deepEqual([...removedIds(['sess-a', 'sess-b'], results)], ['sess-a', 'sess-b']);
});

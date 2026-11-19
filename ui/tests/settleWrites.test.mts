// node --test ui/tests/*.test.mts (Node strips the types).
// Settings save themselves after a quiet moment, and leaving first never
// drops the save. Found by the whole-code review (2026-11-19): Back within
// the quiet time after an edit cleared the only timer, and the edit was lost.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { settleWrites } from '../src/lib/settleWrites.mts';

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

test('the newest write goes once the quiet time is over', async () => {
  const writes = settleWrites(20);
  const done: string[] = [];
  writes.later(() => done.push('first'));
  writes.later(() => done.push('second'));
  assert.deepEqual(done, []);
  await wait(60);
  assert.deepEqual(done, ['second']);
});

test('leaving before the quiet time is over writes at once', async () => {
  const writes = settleWrites(1_000);
  const done: string[] = [];
  writes.later(() => done.push('edit'));
  writes.flush();
  assert.deepEqual(done, ['edit']);
  // Never twice.
  writes.flush();
  await wait(10);
  assert.deepEqual(done, ['edit']);
});

test('nothing left to write: leaving writes nothing', () => {
  const writes = settleWrites(1_000);
  const done: string[] = [];
  writes.later(() => done.push('edit'));
  writes.cancel();
  writes.flush();
  assert.deepEqual(done, []);
});

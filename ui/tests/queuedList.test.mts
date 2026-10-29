// node --test ui/tests/*.test.mts (Node strips the types).
// The queued list the page shows: one agent's queue event never drops
// another agent's rows, and a row hidden for a remove the engine never
// heard comes back where it stood.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { restoreQueued, withAgentQueue, withoutQueued } from '../src/lib/queuedList.mts';

const q = (id: string, agent_id = 'ling') => ({ id, agent_id });

test("one agent's queue event keeps the other agents' rows", () => {
  const list = [q('1', 'ling'), q('2', 'yinyue'), q('3', 'ling')];
  assert.deepEqual(withAgentQueue(list, 'ling', [q('3', 'ling')]), [q('2', 'yinyue'), q('3', 'ling')]);
  assert.deepEqual(withAgentQueue(list, 'yinyue', []), [q('1'), q('3')]);
});

test('a hidden row comes back in its place', () => {
  const list = [q('1'), q('2'), q('3')];
  const { list: hidden, at } = withoutQueued(list, '2');
  assert.deepEqual(hidden, [q('1'), q('3')]);
  assert.equal(at, 1);
  assert.deepEqual(restoreQueued(hidden, q('2'), at), list);
});

test('a row the engine listed again is not doubled', () => {
  const list = [q('1'), q('2')];
  assert.deepEqual(restoreQueued(list, q('2'), 0), list);
});

test('a row whose neighbours left goes back at the end', () => {
  assert.deepEqual(restoreQueued([q('1')], q('3'), 2), [q('1'), q('3')]);
});

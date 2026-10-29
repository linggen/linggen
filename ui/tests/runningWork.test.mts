// node --test ui/tests/*.test.mts (Node strips the types).
// The confirm before a delete names the work it stops, and nothing when
// nothing runs.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { addWork, NO_WORK, runningWorkMessage } from '../src/lib/runningWork.mts';

test('nothing running asks nothing extra', () => {
  assert.equal(runningWorkMessage(NO_WORK), null);
});

test('the work is named with its counts', () => {
  assert.equal(runningWorkMessage({ turn: false, tasks: 2, subagents: 1 }),
    '2 background tasks and 1 subagent are still running. Deleting stops them.');
  assert.equal(runningWorkMessage({ turn: false, tasks: 1, subagents: 0 }),
    '1 background task is still running. Deleting stops it.');
  assert.equal(runningWorkMessage({ turn: false, tasks: 0, subagents: 3 }),
    '3 subagents are still running. Deleting stops them.');
  assert.equal(runningWorkMessage({ turn: true, tasks: 0, subagents: 0 }),
    'The current reply is still running. Deleting stops it.');
  assert.equal(runningWorkMessage({ turn: true, tasks: 1, subagents: 2 }),
    'The current reply, 1 background task and 2 subagents are still running. Deleting stops them.');
});

test('several sessions add up', () => {
  const work = addWork([{ turn: false, tasks: 1, subagents: 0 }, { turn: true, tasks: 1, subagents: 1 }]);
  assert.deepEqual(work, { turn: true, tasks: 2, subagents: 1 });
});

test('each piece goes through the page language', () => {
  const zh: Record<string, string> = { '2 background tasks': '2 个后台任务' };
  const said = runningWorkMessage({ turn: false, tasks: 2, subagents: 0 }, (s) => zh[s] ?? s.toUpperCase());
  assert.equal(said, '2 个后台任务 ARE STILL RUNNING. DELETING STOPS THEM.');
});

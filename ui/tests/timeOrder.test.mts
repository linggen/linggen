import { test } from 'node:test';
import assert from 'node:assert/strict';
import { sortByTime } from '../src/lib/timeOrder.mts';
import type { ChatMessage } from '../src/types.ts';

const row = (sec: number, from: string, text: string, rowOrder?: number, runId?: string): ChatMessage => ({
  role: from === 'user' ? 'user' : 'agent',
  from,
  text,
  timestamp: '',
  timestampMs: sec * 1000,
  ...(rowOrder != null ? { rowOrder } : {}),
  ...(runId ? { runId } : {}),
});

// A merge lists rows without a run id (the person's) before a run's rows, so
// a message the turn took in mid-reply came out above the reply saved in the
// same second. The stored order decides the tie.
test('rows saved in the same second keep the order they were stored in', () => {
  const merged = [
    row(1433, 'user', 'write a long essay', 0),
    row(1451, 'user', 'stop the essay', 2),
    row(1451, 'ling', 'the essay so far', 1, 'r2'),
    row(1453, 'ling', '1455', 3, 'r2'),
  ];
  assert.deepEqual(sortByTime(merged).map((m) => m.text), [
    'write a long essay',
    'the essay so far',
    'stop the essay',
    '1455',
  ]);
});

test('time decides first; ties without a stored place keep their list place', () => {
  const live = [row(5, 'ling', 'b'), row(5, 'user', 'a'), row(3, 'user', 'first', 9), row(0, 'ling', 'no time')];
  assert.deepEqual(sortByTime(live).map((m) => m.text), ['first', 'b', 'a', 'no time']);
});

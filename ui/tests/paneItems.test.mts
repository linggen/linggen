// node --test ui/tests/*.test.mts (Node strips the types).
// The subagent pane lists every subagent and background task once: a
// background subagent's task and tree entry are one row.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { firstLine, formatElapsed, paneItems } from '../src/lib/paneItems.mts';

const entry = (subagentId: string, status: 'running' | 'done' | 'failed', extra = {}) => ({
  subagentId, agentName: 'general-purpose', task: `Look into ${subagentId}\nand more`, status,
  toolCount: 0, contextTokens: 0, currentActivity: null, toolSteps: [], ...extra,
});

test('a background subagent is one item: the entry, with the task beside it', () => {
  const items = paneItems(
    [entry('sub-a', 'running', { timestampMs: 100, currentActivity: 'Reading a.md' })],
    [{ id: 't1', kind: 'subagent', title: 'a', status: 'running', started_at: 90, subagent_id: 'sub-a' }],
  );
  assert.equal(items.length, 1);
  assert.equal(items[0].key, 'sub-a');
  assert.equal(items[0].kind, 'subagent');
  assert.equal(items[0].task?.id, 't1');
  assert.equal(items[0].title, 'Look into sub-a', 'the task line, not the agent type');
  assert.equal(items[0].agentType, 'general-purpose');
  assert.equal(items[0].startedAt, 90);
  assert.equal(items[0].line, 'Reading a.md');
});

test("the task decides a merged item's status and how it ended", () => {
  const [item] = paneItems(
    [entry('sub-a', 'running')],
    [{ id: 't1', kind: 'subagent', title: 'a', status: 'stopped', started_at: 1, ended_at: 5, summary: 'Stopped by you', subagent_id: 'sub-a' }],
  );
  assert.equal(item.status, 'stopped');
  assert.equal(item.running, false);
  assert.equal(item.endedAt, 5);
  assert.equal(item.line, 'Stopped by you');
});

test('shells, foreground subagents and unmatched tasks each stand alone, newest first', () => {
  const items = paneItems(
    [entry('fg', 'done', { timestampMs: 200, resultText: 'All good\nDetails' }), entry('old', 'failed')],
    [
      { id: 'sh', kind: 'shell', title: 'npm test\n--watch', status: 'running', started_at: 300, activity: 'PASS a' },
      { id: 'bg', kind: 'subagent', title: 'Lost one', status: 'completed', started_at: 100, subagent_id: 'gone' },
    ],
  );
  assert.deepEqual(items.map((i) => i.key), ['sh', 'fg', 'bg', 'old']);
  assert.equal(items[0].title, 'npm test');
  assert.equal(items[0].line, 'PASS a');
  assert.equal(items[1].status, 'completed');
  assert.equal(items[1].line, 'All good');
  assert.equal(items[2].kind, 'subagent');
  assert.equal(items[2].entry, undefined);
  assert.equal(items[3].status, 'failed');
});

test('first lines and elapsed times', () => {
  assert.equal(firstLine('  one\ntwo'), 'one');
  assert.equal(firstLine(undefined), '');
  assert.equal(formatElapsed(42_000), '42s');
  assert.equal(formatElapsed(125_000), '2m 5s');
  assert.equal(formatElapsed(3_720_000), '1h 2m');
});

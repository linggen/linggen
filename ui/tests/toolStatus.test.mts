// node --test ui/tests/*.test.mts (Node strips the types).
// A tool call's status reads the same live and after a reload: the engine's
// word, from its block update or from the calls a history row carries.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { savedToolBlocks, toolStatus } from '../src/lib/toolStatus.mts';

test('live: the engine words a block update with its status', () => {
  assert.equal(toolStatus('running'), 'running');
  assert.equal(toolStatus('done'), 'done');
  assert.equal(toolStatus('failed'), 'failed');
  assert.equal(toolStatus(null), undefined);
  assert.equal(toolStatus('weird'), undefined);
});

test('saved: each call keeps the status it ended in', () => {
  const blocks = savedToolBlocks([
    { tool: 'Read', args: '{"path":"a.md"}', status: 'done' },
    { tool: 'Write', args: '{"path":"probe.txt"}', status: 'failed' },
    { tool: 'Bash', args: '{"cmd":"sleep 120"}', status: 'failed' },
  ], 'r1');
  assert.deepEqual(
    blocks.map((b) => [b.type, b.tool, b.args, b.status]),
    [
      ['tool_use', 'Read', '{"path":"a.md"}', 'done'],
      ['tool_use', 'Write', '{"path":"probe.txt"}', 'failed'],
      ['tool_use', 'Bash', '{"cmd":"sleep 120"}', 'failed'],
    ],
  );
  assert.deepEqual(blocks.map((b) => b.id), ['saved-r1-0', 'saved-r1-1', 'saved-r1-2']);
});

test('a saved call is over: no word, or "running", never ended', () => {
  const blocks = savedToolBlocks([{ tool: 'Look' }, { tool: 'Bash', status: 'running' }], 'x');
  assert.deepEqual(blocks.map((b) => [b.args, b.status]), [['', 'failed'], ['', 'failed']]);
});

test('a row with no calls has no tool blocks', () => {
  assert.deepEqual(savedToolBlocks(undefined, 'x'), []);
  assert.deepEqual(savedToolBlocks([], 'x'), []);
});

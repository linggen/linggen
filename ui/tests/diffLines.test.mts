// node --test ui/tests/*.test.mts (Node strips the types).
// Inside a hunk every line is the hunk's. Found by the whole-code review
// (2026-11-19): every line starting `+++ ` or `--- ` was skipped as a file
// header, inside a hunk too — an added `++ x` line vanished and the line
// numbers after it shifted.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseDiff } from '../src/lib/diffLines.mts';
import { diffStats } from '../src/lib/diffStats.mts';

const diff = [
  'diff --git a/notes.md b/notes.md',
  'index 1111111..2222222 100644',
  '--- a/notes.md',
  '+++ b/notes.md',
  '@@ -10,3 +10,3 @@',
  ' keep',
  '--- old rule',
  '+++ new rule',
  ' tail',
].join('\n');

test('a hunk line that reads like a header is a change', () => {
  const lines = parseDiff(diff);
  assert.deepEqual(lines.map((l) => l.type), ['hunk', 'context', 'del', 'add', 'context']);
  assert.deepEqual(lines[2], { type: 'del', oldNum: 11, text: '-- old rule' });
  assert.deepEqual(lines[3], { type: 'add', newNum: 11, text: '++ new rule' });
  // The numbers after it stay right.
  assert.deepEqual(lines[4], { type: 'context', oldNum: 12, newNum: 12, text: 'tail' });
});

test('the next file\'s headers after a hunk are headers', () => {
  const two = `${diff}\n--- a/other.md\n+++ b/other.md\n@@ -1 +1 @@\n-a\n+b`;
  const lines = parseDiff(two);
  assert.deepEqual(lines.slice(5).map((l) => [l.type, l.text]), [['hunk', '@@ -1 +1 @@'], ['del', 'a'], ['add', 'b']]);
});

test('the +/- counts read the same lines', () => {
  assert.deepEqual(diffStats(diff), { added: 1, deleted: 1 });
});

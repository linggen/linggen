// node --test ui/tests/*.test.mts
// The Memory tab's editor: what an edit changes, and how rows are worded.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { draftOf, patchOf, problemWith } from '../src/components/memory/memoryDraft.mts';
import { fmtBytes, preview, relAge, shortPath, tierLabel } from '../src/components/memory/memoryFormat.mts';
import type { MemoryRow } from '../src/components/memory/memoryApi.mts';

const row: MemoryRow = {
  id: 'r1', content: 'Harbor code always uses tabs.', type: 'preference', tier: 'semantic', from: 'user',
  scope: '/Users/alex/harbor', summary: 'Harbor: tabs', indexed: true,
};

test('an untouched draft changes nothing', () => {
  assert.equal(patchOf(row, draftOf(row)), null);
});

test('only the fields that differ go in the patch', () => {
  const draft = { ...draftOf(row), content: 'Harbor uses tabs.', tier: 'core' as const, indexed: false };
  assert.deepEqual(patchOf(row, draft), { content: 'Harbor uses tabs.', tier: 'core', indexed: false });
});

test('emptying a nullable field clears it; filling one sets it', () => {
  const cleared = patchOf(row, { ...draftOf(row), summary: '', scope: '  ' });
  assert.deepEqual(cleared, { summary: null, scope: null });
  const set = patchOf(row, { ...draftOf(row), outcome: 'positive' });
  assert.deepEqual(set, { outcome: 'positive' });
});

test('a row with no content cannot be saved', () => {
  assert.equal(problemWith({ ...draftOf(row), content: '   ' }), 'Content is required.');
  assert.equal(problemWith(draftOf(row)), null);
});

test('ages, sizes and paths read plainly', () => {
  const now = Date.parse('2026-11-16T12:00:00Z');
  assert.equal(relAge('2026-11-16T11:59:30Z', now), 'just now');
  assert.equal(relAge('2026-11-16T09:00:00Z', now), '3h ago');
  assert.equal(relAge('2026-11-13T12:00:00Z', now), '3d ago');
  assert.equal(relAge(null, now), '');
  assert.equal(fmtBytes(2_500_000), '2.5 MB');
  assert.equal(shortPath('/Users/alex/harbor'), '~/harbor');
  assert.equal(tierLabel('semantic'), 'Long-term');
  assert.equal(preview('a   b\n c', 80), 'a b c');
  assert.equal(preview('x'.repeat(300), 10).length, 10);
});

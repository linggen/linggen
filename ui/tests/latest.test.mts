// node --test ui/tests/*.test.mts (Node strips the types).
// The newest request wins. Found by the whole-code review (2026-11-19): a
// file read of A that answered after B was picked showed A's text under B,
// and the next Save wrote it into B; an older search or memory filter
// answered over a newer one.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { latestOnly } from '../src/lib/latest.mts';

test('an answer that comes back after a newer request is dropped', () => {
  const reads = latestOnly();
  const readA = reads.begin();
  const readB = reads.begin();
  // B answers first, then A late.
  assert.equal(readB(), true);
  assert.equal(readA(), false);
});

test('the only request is current until another starts', () => {
  const reads = latestOnly();
  const only = reads.begin();
  assert.equal(only(), true);
  assert.equal(only(), true);
  reads.begin();
  assert.equal(only(), false);
});

test('two sources do not cancel each other', () => {
  const files = latestOnly();
  const search = latestOnly();
  const file = files.begin();
  search.begin();
  assert.equal(file(), true);
});

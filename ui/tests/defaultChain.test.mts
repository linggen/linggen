// node --test ui/tests/*.test.mts (Node strips the types).
// Clear on the default model takes that model off the chain. Found by the
// whole-code review (2026-11-19): with fallbacks after it, Clear removed all
// the fallbacks and kept the model cleared.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { toggledDefault } from '../src/lib/defaultChain.mts';

test('clearing the default keeps its fallbacks', () => {
  assert.deepEqual(toggledDefault(['a', 'b', 'c'], 'a'), ['b', 'c']);
});

test('clearing the only default leaves none', () => {
  assert.deepEqual(toggledDefault(['a'], 'a'), []);
});

test('another model becomes the one default', () => {
  assert.deepEqual(toggledDefault([], 'a'), ['a']);
  assert.deepEqual(toggledDefault(['a', 'b'], 'c'), ['c']);
  assert.deepEqual(toggledDefault(['a', 'b'], 'b'), ['b']);
});

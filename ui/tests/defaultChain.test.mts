// node --test ui/tests/*.test.mts (Node strips the types).
// Making a model the default keeps the chain after it as fallbacks: the old
// star path replaced the whole chain, so a second pick dropped every fallback.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { madeDefault } from '../src/lib/defaultChain.mts';

test('a model made the default goes first and the rest stay as fallbacks', () => {
  assert.deepEqual(madeDefault(['a', 'b'], 'c'), ['c', 'a', 'b']);
  assert.deepEqual(madeDefault(['c', 'a', 'b'], 'b'), ['b', 'c', 'a']);
});

test('a model already first, or a chain with nothing, stays whole', () => {
  assert.deepEqual(madeDefault(['b', 'c', 'a'], 'b'), ['b', 'c', 'a']);
  assert.deepEqual(madeDefault([], 'a'), ['a']);
});

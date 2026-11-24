// node --test ui/tests/*.test.mts
// The chat's thinking-effort chip lists only what the engine says the model
// offers, hides when it offers none, and never erases a kept value.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { EFFORT_WORD, effortChip } from '../src/lib/effortChip.mts';

const models = [
  { id: 'sonnet', effort: ['low', 'medium', 'high', 'xhigh', 'max'] },
  { id: 'opus-4-5', effort: ['low', 'medium', 'high'] },
  { id: 'local', effort: [] },
  { id: 'old' },
];

test('the chip lists the values the model offers, in the engine order', () => {
  assert.deepEqual(effortChip(models, 'sonnet', null)?.values, ['low', 'medium', 'high', 'xhigh', 'max']);
  assert.deepEqual(effortChip(models, 'opus-4-5', null)?.values, ['low', 'medium', 'high']);
});

test('a model that offers none, or no model, has no chip', () => {
  for (const id of ['local', 'old', 'missing', null, undefined]) assert.equal(effortChip(models, id, 'high'), null);
});

test('automatic is selected until the model offers the kept value', () => {
  assert.equal(effortChip(models, 'sonnet', null)?.selected, '');
  assert.equal(effortChip(models, 'sonnet', 'xhigh')?.selected, 'xhigh');
  // The choice is kept by the session, only not shown where it does not apply.
  assert.equal(effortChip(models, 'opus-4-5', 'xhigh')?.selected, '');
});

test('every value has its Chinese word, short', () => {
  const zh: Record<string, string> = new Function(
    readFileSync(new URL('../src/lib/strings-zh.ts', import.meta.url), 'utf8').replace(/^export const ZH[^=]*=/m, 'return '),
  )();
  const said = Object.values(EFFORT_WORD).map((word) => zh[word]);
  assert.deepEqual(said, ['低', '中', '高', '超高', '最高', '极限']);
  assert.equal(zh['Thinking'], '思考');
});

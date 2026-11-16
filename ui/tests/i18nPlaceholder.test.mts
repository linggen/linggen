import { test } from 'node:test';
import assert from 'node:assert/strict';
import { translatePlaceholder } from '../src/lib/placeholder.mts';

const dictionary: Record<string, string> = { 'Search by meaning': '按意思搜索' };
const translate = (text: string) => dictionary[text] ?? null;

test('placeholders use the dictionary unless explicitly opted out', () => {
  assert.equal(translatePlaceholder('Search by meaning', false, translate), '按意思搜索');
  assert.equal(translatePlaceholder('Search by meaning', true, translate), null);
  assert.equal(translatePlaceholder('Unknown', false, translate), null);
});

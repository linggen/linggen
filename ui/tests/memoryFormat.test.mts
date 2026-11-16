// node --test ui/tests/*.test.mts
// How the Memory tab words counts and dates.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { rowCount, uiLocale } from '../src/components/memory/memoryFormat.mts';

test('a count says "1 row", never "1 rows"', () => {
  assert.equal(rowCount(0), '0 rows');
  assert.equal(rowCount(1), '1 row');
  assert.equal(rowCount(2), '2 rows');
});

test('dates follow the page language, and the browser when there is no page', () => {
  assert.equal(uiLocale(), undefined);
  const doc = { documentElement: { lang: 'zh-Hans' } };
  Object.defineProperty(globalThis, 'document', { value: doc, configurable: true });
  assert.equal(uiLocale(), 'zh-CN');
  doc.documentElement.lang = 'en';
  assert.equal(uiLocale(), undefined);
  delete (globalThis as { document?: unknown }).document;
});

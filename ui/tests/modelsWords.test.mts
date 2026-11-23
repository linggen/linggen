// node --test ui/tests/*.test.mts (Node strips the types).
// Settings → Models says only the words of lib/modelsWords.mts; each must have
// its 简体中文 and be short (twenty words at most, as every message is).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { W, say } from '../src/lib/modelsWords.mts';
import { readFileSync } from 'node:fs';
import { translateFrom } from '../src/lib/i18nMatch.mts';

// The table is a .ts file the page imports; here it is read as text, as the other tests do.
const ZH: Record<string, string> = new Function(
  readFileSync(new URL('../src/lib/strings-zh.ts', import.meta.url), 'utf8').replace(/^export const ZH[^=]*=/m, 'return '),
)();

test('every word of the Models page has its Chinese', () => {
  const missing = Object.values(W).filter((word) => ZH[word] === undefined);
  assert.deepEqual(missing, []);
});

test('no word is longer than twenty words', () => {
  const long = Object.values(W).filter((word) => word.split(/\s+/).length > 20);
  assert.deepEqual(long, []);
});

test('a slot of the English is a slot of the Chinese', () => {
  for (const word of Object.values(W)) {
    const slots = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
    assert.deepEqual(slots(ZH[word]), slots(word), word);
  }
});

test('a sentence with its slots filled is put into Chinese by the page', () => {
  assert.equal(translateFrom(ZH, say(W.now, { name: 'Claude Opus 5.5' })), '现在是 Claude Opus 5.5');
  assert.equal(translateFrom(ZH, say(W.priceIn, { input: '4', output: '20' })), '输入 $4 / 输出 $20 每百万');
  assert.equal(translateFrom(ZH, say(W.moreN, { n: 7 })), '更多（7）');
});

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { W, say } from '../src/lib/modelsWords.mts';
import { translateFrom } from '../src/lib/i18nMatch.mts';
const ZH: Record<string, string> = new Function(
  readFileSync(new URL('../src/lib/strings-zh.ts', import.meta.url), 'utf8').replace(/^export const ZH[^=]*=/m, 'return '),
)();
test('the download progress line reads in Chinese', () => {
  const out = translateFrom(ZH, say(W.downloading, { model: 'gemma2:latest', progress: '1% · 64 MB / 5.4 GB' }));
  assert.equal(out, '正在下载 gemma2:latest… 1% · 64 MB / 5.4 GB');
});

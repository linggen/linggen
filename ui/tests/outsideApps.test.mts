// Settings → Tools & MCP → Outside agents: what the section says (lib/outsideApps.mts)
// and the one rule the page keeps — a switch sends the whole list of open apps.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { WORDS, openAfterToggle, type OutsideApp } from '../src/lib/outsideApps.mts';
import { translateFrom } from '../src/lib/i18nMatch.mts';

const ZH: Record<string, string> = new Function(
  readFileSync(new URL('../src/lib/strings-zh.ts', import.meta.url), 'utf8').replace(/^export const ZH[^=]*=/m, 'return '),
)();

test('the section says twenty words in all, each with a Chinese entry', () => {
  const all = Object.values(WORDS);
  assert.ok(all.join(' ').split(/\s+/).length <= 20, `too long: ${all.join(' ')}`);
  for (const text of all) {
    const zh = translateFrom(ZH, text);
    assert.ok(zh && /\p{Script=Han}/u.test(zh), `no Chinese for: ${text}`);
  }
});

const apps: OutsideApp[] = [
  { name: 'vault', description: 'Keeps the money.', open: true },
  { name: 'diary', description: 'Keeps the days.', open: false },
];

test('a switch sends every open app, with the one moved changed', () => {
  assert.deepEqual(openAfterToggle(apps, 'diary', true), ['vault', 'diary']);
  assert.deepEqual(openAfterToggle(apps, 'vault', false), []);
  assert.deepEqual(openAfterToggle(apps, 'vault', true), ['vault']);
});

// Settings → People: the rules the page keeps (lib/people.mts) and what it says.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import {
  LEVELS, WORDS, allWords, appRows, appsAfterToggle, arrivalLine, deviceLabel, limitFromText, limitText,
  modelRows, modelsAfterToggle, offeredApps, usedLine,
} from '../src/lib/people.mts';
import { translateFrom } from '../src/lib/i18nMatch.mts';

const ZH: Record<string, string> = new Function(
  readFileSync(new URL('../src/lib/strings-zh.ts', import.meta.url), 'utf8').replace(/^export const ZH[^=]*=/m, 'return '),
)();

const installed = [
  { name: 'dice', label: 'Dice' },
  { name: 'music', label: 'Music' },
  { name: 'money', label: 'Money' },
];

test('every sentence is at most twenty words and has a Chinese entry', () => {
  for (const text of allWords()) {
    assert.ok(text.split(/\s+/).length <= 20, `too long: ${text}`);
  }
  const shown = [...allWords(), ...LEVELS.map((l) => l.label), usedLine(1200, null), usedLine(1200, 200000)];
  for (const text of shown) {
    const zh = translateFrom(ZH, text);
    assert.ok(zh && /\p{Script=Han}/u.test(zh), `no Chinese for: ${text}`);
  }
  assert.equal(translateFrom(ZH, usedLine(1200, 200000)), '今日已用：1,200／200,000');
});

test('three levels are offered, admin never', () => {
  assert.deepEqual(LEVELS.map((l) => l.id), ['chat', 'read', 'edit']);
});

test('apps: all means every app the owner has not kept; a kept app is shown, never a choice', () => {
  const rows = appRows(installed, ['money'], null);
  assert.deepEqual(rows.map((r) => [r.name, r.on, r.kept]), [['dice', true, false], ['music', true, false], ['money', false, true]]);
  assert.deepEqual(offeredApps(installed, ['money']).map((a) => a.name), ['dice', 'music']);
});

test('apps: switching one off from "all" lists the others; an app no longer installed stays, to be removed', () => {
  const offered = offeredApps(installed, ['money']);
  assert.deepEqual(appsAfterToggle(null, offered, 'music'), ['dice']);
  assert.deepEqual(appsAfterToggle(['dice'], offered, 'music'), ['dice', 'music']);
  const rows = appRows(installed, [], ['dice', 'old-app']);
  assert.deepEqual(rows.filter((r) => r.on).map((r) => [r.name, r.gone]), [['dice', false], ['old-app', true]]);
  assert.deepEqual(appsAfterToggle(['dice', 'old-app'], offered, 'old-app'), ['dice']);
});

test('models: none is a valid state; one this Mac no longer has stays, to be removed', () => {
  assert.deepEqual(modelRows(['a', 'b'], []).map((r) => r.on), [false, false]);
  assert.deepEqual(modelRows(['a'], ['a', 'old']).map((r) => [r.id, r.on, r.gone]), [['a', true, false], ['old', true, true]]);
  assert.deepEqual(modelsAfterToggle(['a'], 'b'), ['a', 'b']);
  assert.deepEqual(modelsAfterToggle(['a', 'b'], 'a'), ['b']);
  assert.deepEqual(modelsAfterToggle(['a'], 'a'), []);
  assert.equal(WORDS.noModel, 'No model of this Mac — they use their own');
});

test('a limit: blank is none, a whole number is a limit, anything else is refused', () => {
  assert.deepEqual(limitFromText(''), { limit: null });
  assert.deepEqual(limitFromText('  '), { limit: null });
  assert.deepEqual(limitFromText('200000'), { limit: 200000 });
  assert.deepEqual(limitFromText('200,000'), { limit: 200000 });
  for (const text of ['abc', '-5', '1.5', '2k', '1e6']) assert.deepEqual(limitFromText(text), { bad: true });
  assert.equal(limitText(null), '');
  assert.equal(limitText(undefined), '');
  assert.equal(limitText(200000), '200000');
});

test('what was used today is shown with or without a limit', () => {
  assert.equal(usedLine(0, null), 'Used today: 0');
  assert.equal(usedLine(1200, 200000), 'Used today: 1,200 of 200,000');
});

test('a sign-in that moved or kept settings is told once, in one line; none says nothing', () => {
  assert.equal(arrivalLine(undefined), null);
  assert.equal(arrivalLine('moved'), WORDS.arrival.moved);
  assert.equal(arrivalLine('kept'), WORDS.arrival.kept);
  assert.notEqual(WORDS.arrival.moved, WORDS.arrival.kept);
});

test('a device under a person is named with its kind', () => {
  assert.equal(deviceLabel({ id: '1', name: "Friend's phone", kind: 'phone' }), "Friend's phone · Phone");
  assert.equal(deviceLabel({ id: '2', name: 'Studio', kind: 'machine' }), 'Studio · Computer');
  assert.equal(translateFrom(ZH, deviceLabel({ id: '1', name: "Friend's phone", kind: 'phone' })), "Friend's phone · 手机");
  assert.equal(translateFrom(ZH, deviceLabel({ id: '2', name: 'Studio', kind: 'machine' })), 'Studio · 电脑');
});

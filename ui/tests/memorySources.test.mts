// Settings → Notes: the rules the page keeps (lib/memorySources.mts) and what it says.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import {
  OWN_SOURCE, WORDS, actionLabel, allWords, countLine, dayOf, fileCountLine, noteActions, noteMeta, refusalCode, refusalLine,
  reviewLine, sizeLine,
} from '../src/lib/memorySources.mts';
import { SETTINGS_SECTIONS } from '../src/lib/settingsSections.mts';
import { longPress, DRIFT_PX, HOLD_MS, type Clock } from '../src/lib/longPress.mts';
import { translateFrom } from '../src/lib/i18nMatch.mts';

const ZH: Record<string, string> = new Function(
  readFileSync(new URL('../src/lib/strings-zh.ts', import.meta.url), 'utf8').replace(/^export const ZH[^=]*=/m, 'return '),
)();

test('every sentence is at most twenty words and has a Chinese entry', () => {
  const shown = [...allWords(), countLine(1), countLine(4), fileCountLine(1), fileCountLine(3)];
  for (const text of shown) {
    assert.ok(text.split(/\s+/).length <= 20, `too long: ${text}`);
    const zh = translateFrom(ZH, text);
    assert.ok(zh && /\p{Script=Han}/u.test(zh), `no Chinese for: ${text}`);
  }
  assert.equal(translateFrom(ZH, countLine(4)), '4 条笔记');
});

test('Notes is a declared section of Data, drawn by the page, and its label avoids "Memory"', () => {
  const notes = SETTINGS_SECTIONS.find((s) => s.id === 'notes');
  assert.ok(notes);
  assert.equal(notes.group, 'Data');
  // The Memory section's own specs find its button by that word.
  assert.doesNotMatch(notes.label, /memory/i);
  assert.match(readFileSync(new URL('../src/components/settings/sections.tsx', import.meta.url), 'utf8'), /MemorySourcesTab/);
});

test('one menu: edit first, delete last, void and un-void and the two raise words swap', () => {
  const plain = { void: null, no_raise: false };
  assert.deepEqual(noteActions(plain), ['edit', 'void', 'no-raise', 'delete']);
  assert.deepEqual(noteActions({ void: 'wrong', no_raise: true }), ['edit', 'unvoid', 'raise', 'delete']);
  for (const action of noteActions(plain).concat(noteActions({ void: 'x', no_raise: true }))) {
    assert.ok(actionLabel(action));
  }
});

test('the page puts a refusal in its own words, never the engine sentence', () => {
  assert.equal(refusalCode('{"error":"conflict","detail":"note abc changed since you read it (now: …)"}'), 'conflict');
  assert.equal(refusalCode('HTTP 500'), undefined);
  assert.equal(refusalLine('conflict'), WORDS.refused.conflict);
  assert.equal(refusalLine('something_new'), WORDS.refusedOther);
  assert.equal(refusalLine(undefined), WORDS.refusedOther);
});

test('lines of a note and a file', () => {
  assert.equal(noteMeta({ kind: 'rule', source: 'codex', updated: '2026-11-24' }), 'rule · codex · 2026-11-24');
  assert.equal(reviewLine({ why: 'unused', void: null }), WORDS.unused);
  assert.equal(reviewLine({ why: 'void', void: 'no longer true' }), 'Marked wrong: no longer true');
  assert.equal(sizeLine(512), '512 B');
  assert.equal(sizeLine(2048), '2.0 KB');
  assert.equal(sizeLine(20480), '20 KB');
  assert.match(dayOf(1795608000), /^2026-11-2[345]$/);
  assert.equal(OWN_SOURCE, 'linggen');
});

test('the page changes notes only through the one api object, and names no write route for another source', () => {
  const dir = new URL('../src/components/memorySources/', import.meta.url);
  for (const file of ['OtherAgentNotes.tsx']) {
    const text = readFileSync(new URL(file, dir), 'utf8');
    assert.doesNotMatch(text, /memorySourcesApi\.(edit|void|unvoid|noRaise|remove)|useNoteActions|apiPut|apiPost|apiDelete/, `${file} can change a note`);
  }
  const endpoints = readFileSync(new URL('../src/lib/endpoints.ts', import.meta.url), 'utf8');
  // The changes name a note and no source: nothing of another agent's can be addressed.
  assert.doesNotMatch(endpoints, /remembered-sources\/[^`]*`,\s*\{/);
});

function fakeClock() {
  let next: (() => void) | null = null;
  let cleared = 0;
  const clock: Clock = { set: (run) => { next = run; return 1; }, clear: () => { next = null; cleared += 1; } };
  return { clock, fire: () => next?.(), pending: () => next !== null, cleared: () => cleared };
}

test('a held press opens the menu; lifting, drifting or a second press does not', () => {
  const held: string[] = [];
  const t = fakeClock();
  const press = longPress(() => held.push('menu'), t.clock);
  assert.equal(HOLD_MS, 500);

  press.down(10, 10);
  t.fire();
  assert.deepEqual(held, ['menu']);

  press.down(10, 10);
  press.up();
  assert.equal(t.pending(), false);

  press.down(10, 10);
  press.move(10 + DRIFT_PX, 10);
  assert.equal(t.pending(), true, 'a small tremor is still a hold');
  press.move(10 + DRIFT_PX + 1, 10);
  assert.equal(t.pending(), false, 'a scroll is not a hold');
  assert.deepEqual(held, ['menu']);
});

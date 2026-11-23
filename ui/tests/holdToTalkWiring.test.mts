// How Yinyue's avatar binds the shared hold-to-talk control (shared/hold-to-talk.js,
// tested in tests/shared/hold-to-talk.test.mjs): her one stop, her sound while a
// hold is on, and the one copy the UI imports. Reads the page's source.
import assert from 'node:assert/strict';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { voicedHere } from '../src/lib/petCue.mts';

const src = fileURLToPath(new URL('../src', import.meta.url));
function* files(dir: string): Generator<string> {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) yield* files(p);
    else if (/\.(ts|tsx|mts|js|jsx)$/.test(name)) yield p;
  }
}
const withText = (needle: string) => [...files(src)].filter((f) => readFileSync(f, 'utf8').includes(needle)).map((f) => relative(src, f));

test('lock: the avatar takes the control from the one shared file, through the alias', () => {
  const wiring = readFileSync(join(src, 'lib/yinyueTalk.ts'), 'utf8');
  assert.match(wiring, /from '@linggen\/hold-to-talk'/);
  assert.deepEqual(withText('createHoldToTalk').filter((f) => f !== 'types/hold-to-talk.d.ts'), ['lib/yinyueTalk.ts']);
});

test('lock: her mute and her cut-off by a press are the same stop', () => {
  const wiring = readFileSync(join(src, 'lib/yinyueTalk.ts'), 'utf8');
  assert.match(wiring, /cutOff: hush\b/);
  const handlers = readFileSync(join(src, 'lib/eventHandlers/yinyue.ts'), 'utf8');
  assert.equal((handlers.match(/function hush\(/g) ?? []).length, 1, 'one stop function');
  assert.match(handlers, /export function hush\(/);
});

test('rule 5: her line is voiced unless it is muted or the person is speaking to her', () => {
  assert.equal(voicedHere(true, false), true);
  assert.equal(voicedHere(undefined, false), true);
  assert.equal(voicedHere(false, false), false, 'muted, or seen where it shows');
  assert.equal(voicedHere(true, true), false, 'a hold is on: shown, not voiced');
  assert.equal(voicedHere(undefined, true), false);
});

test('lock rule 5: the page decides her sound in handlePetSpeak, and a hold sets the flag', () => {
  const handlers = readFileSync(join(src, 'lib/eventHandlers/yinyue.ts'), 'utf8');
  assert.equal((handlers.match(/voicedHere\(/g) ?? []).length, 1, 'one call decides sound');
  assert.match(handlers, /if \(!voicedHere\(item\.data\?\.voice, personTalking\)\)/);
  const wiring = readFileSync(join(src, 'lib/yinyueTalk.ts'), 'utf8');
  assert.match(wiring, /setPersonTalking\(holding\)/);
  assert.deepEqual(withText('setPersonTalking('), ['lib/eventHandlers/yinyue.ts', 'lib/yinyueTalk.ts']);
});

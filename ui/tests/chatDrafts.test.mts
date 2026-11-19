// node --test ui/tests/*.test.mts (Node strips the types).
// A draft belongs to its session. Found by the whole-code review
// (2026-11-19): the chat box kept its draft, pictures and addressee when the
// session changed, so a draft written in one session was sent into another.
// And on a loaded machine (2026-11-19): words typed while New chat was being
// made went to the session before it when the new one arrived.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { boxSlot, keepDraft, openDraft } from '../src/lib/chatDrafts.mts';

const note = { text: 'A note for A.', images: [{ id: 0, data: 'data:image/png;base64,AA' }], addressee: 'yinyue' };
const empty = { text: '', images: [], addressee: '' };

test("a session's draft is its own", () => {
  openDraft('sess-a');
  keepDraft('sess-a', note);
  assert.equal(openDraft('sess-b'), undefined);
  assert.deepEqual(openDraft('sess-a'), note);
});

test('an empty box keeps no draft', () => {
  openDraft('sess-c');
  keepDraft('sess-c', note);
  keepDraft('sess-c', empty);
  assert.equal(openDraft('sess-c'), undefined);
  // A chip alone is no draft (the pre-fill sets it again).
  keepDraft('sess-c', { ...empty, addressee: 'yinyue' });
  assert.equal(openDraft('sess-c'), undefined);
});

test('while New chat is being made the box is the new chat\'s', () => {
  assert.equal(boxSlot('sess-a', true), '');
  assert.equal(boxSlot('sess-a', false), 'sess-a');
  assert.equal(boxSlot(null, false), '');
});

test('words typed while New chat is made go to the new session', () => {
  openDraft('sess-a');
  keepDraft('sess-a', note);
  // New chat: a fresh, empty box.
  assert.equal(openDraft(''), undefined);
  keepDraft('', { ...empty, text: 'Say hello.' });
  // The new session arrives: its box opens on those words, twice too.
  assert.equal(openDraft('sess-new')?.text, 'Say hello.');
  assert.equal(openDraft('sess-new')?.text, 'Say hello.');
  // The session before keeps its own.
  assert.deepEqual(openDraft('sess-a'), note);
});

test('a New chat opens empty, not on an old unsent one', () => {
  openDraft('');
  keepDraft('', { ...empty, text: 'Left behind.' });
  openDraft('sess-e');
  openDraft('sess-f');
  assert.equal(openDraft(''), undefined);
});

test("another session opened later never takes a new chat's words", () => {
  openDraft('sess-g');
  openDraft('');
  keepDraft('', { ...empty, text: 'For the new one.' });
  openDraft('sess-new2');
  assert.equal(openDraft('sess-h'), undefined);
});

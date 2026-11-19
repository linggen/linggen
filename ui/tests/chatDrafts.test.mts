// node --test ui/tests/*.test.mts (Node strips the types).
// A draft belongs to its session. Found by the whole-code review
// (2026-11-19): the chat box kept its draft, pictures and addressee when the
// session changed, so a draft written in one session was sent into another.
// And on a loaded machine (2026-11-19): words typed while New chat was being
// made were lost when the new session arrived.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { boxSlot, switchDraft } from '../src/lib/chatDrafts.mts';

const note = { text: 'A note for A.', images: [{ id: 0, data: 'data:image/png;base64,AA' }], addressee: 'yinyue' };
const empty = { text: '', images: [], addressee: '' };

test("moving to another session keeps the draft for its own", () => {
  assert.deepEqual(switchDraft('sess-a', 'sess-b', note), empty);
  assert.deepEqual(switchDraft('sess-b', 'sess-a', empty), note);
});

test('an empty box keeps no draft', () => {
  switchDraft('sess-c', 'sess-x', note);
  switchDraft('sess-x', 'sess-c', empty);
  // Emptied, then left: nothing waits.
  switchDraft('sess-c', 'sess-x', empty);
  assert.deepEqual(switchDraft('sess-x', 'sess-c', empty), empty);
  // A chip alone is no draft (the pre-fill sets it again).
  switchDraft('sess-c', 'sess-x', { ...empty, addressee: 'yinyue' });
  assert.deepEqual(switchDraft('sess-x', 'sess-c', empty), empty);
});

test("while New chat is being made the box is the new chat's", () => {
  assert.equal(boxSlot('sess-a', true), '');
  assert.equal(boxSlot('sess-a', false), 'sess-a');
  assert.equal(boxSlot(null, false), '');
});

test('New chat opens empty; the session before keeps its draft', () => {
  assert.deepEqual(switchDraft('sess-d', '', note), empty);
  assert.deepEqual(switchDraft('', 'sess-e', empty), empty);
  assert.deepEqual(switchDraft('sess-e', 'sess-d', empty), note);
});

test('words typed while New chat is made stay when it becomes a session', () => {
  const typed = { ...empty, text: 'Say hello.' };
  assert.deepEqual(switchDraft('', 'sess-new', typed), typed);
});

test('a page not yet on a session keeps its words when it opens one', () => {
  const typed = { ...empty, text: 'Early words.' };
  assert.deepEqual(switchDraft('', 'sess-f', typed), typed);
});

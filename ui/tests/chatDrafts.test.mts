// node --test ui/tests/*.test.mts (Node strips the types).
// A draft belongs to its session. Found by the whole-code review
// (2026-11-19): the chat box kept its draft, pictures and addressee when the
// session changed, so a draft written in one session was sent into another.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { draftOf, keepDraft } from '../src/lib/chatDrafts.mts';

const note = { text: 'A note for A.', images: [{ id: 0, data: 'data:image/png;base64,AA' }], addressee: 'yinyue' };
const empty = { text: '', images: [], addressee: '' };

test("a session's draft is its own", () => {
  keepDraft('sess-a', note);
  assert.deepEqual(draftOf('sess-a'), note);
  assert.equal(draftOf('sess-b'), undefined);
  assert.equal(draftOf(null), undefined);
});

test('an empty box keeps no draft', () => {
  keepDraft('sess-c', note);
  keepDraft('sess-c', empty);
  assert.equal(draftOf('sess-c'), undefined);
  // A chip alone is no draft (the pre-fill sets it again).
  keepDraft('sess-c', { ...empty, addressee: 'yinyue' });
  assert.equal(draftOf('sess-c'), undefined);
});

test('a new chat (no session yet) has its own draft', () => {
  keepDraft(null, { ...note, text: 'Not yet sent.' });
  assert.equal(draftOf(undefined)?.text, 'Not yet sent.');
  assert.equal(draftOf('sess-d'), undefined);
});

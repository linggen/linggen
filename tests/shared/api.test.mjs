// Moved from skills/lingjing/tests/session.test.mjs when the helper moved
// into the engine's /shared/api.js. Run: node --test tests/shared/*.test.mjs
// Which chat the page picks up when the app opens. A session the engine made
// but nobody ever spoke in is not a day to resume: resuming it leaves the
// player looking at an empty panel with no greeting and no way in, which is
// exactly what happened on 2026-09-18.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { pickResumable } from '../../shared/api.js';

const NOW = 1789700000;
const hoursAgo = h => NOW - h * 3600;
const spoken = (id, created, forSec = 60) => ({ id, created_at: created, updated_at: created + forSec });
const empty = (id, created) => ({ id, created_at: created, updated_at: created });

test('the newest chat spoken in today is picked up', () => {
  assert.equal(pickResumable([spoken('old', hoursAgo(5)), spoken('new', hoursAgo(1))], NOW), 'new');
});

test('a session created and never spoken in is passed over', () => {
  assert.equal(pickResumable([empty('ghost', hoursAgo(1)), spoken('real', hoursAgo(4))], NOW), 'real');
});

test('nothing but empty sessions begins a fresh chat, which greets', () => {
  assert.equal(pickResumable([empty('ghost', hoursAgo(1)), empty('ghost2', hoursAgo(2))], NOW), null);
});

test('a day old is a new day', () => {
  assert.equal(pickResumable([spoken('yesterday', hoursAgo(25))], NOW), null);
  assert.equal(pickResumable([spoken('tonight', hoursAgo(23))], NOW), 'tonight');
});

test('no sessions at all', () => {
  assert.equal(pickResumable([], NOW), null);
  assert.equal(pickResumable(undefined, NOW), null);
});

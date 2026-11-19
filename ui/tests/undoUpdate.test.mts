// node --test ui/tests/*.test.mts (Node strips the types).
// A room setting that failed to save goes back to what it was. Found by the
// whole-code review (2026-11-19): the room's switch and sharing scope
// changed on screen first and stayed changed when the save failed.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { undoUpdate } from '../src/lib/undoUpdate.mts';

const before = { room_enabled: true, shared_models: ['a'] };

test('a failed save takes its change back', () => {
  const shown = { ...before, room_enabled: false };
  assert.deepEqual(undoUpdate(shown, before, { room_enabled: false }), before);
});

test('a newer change to the same field stands', () => {
  const shown = { ...before, room_enabled: true, shared_models: ['a', 'b'] };
  // The switch was turned off, then on again; the off save failed.
  assert.deepEqual(undoUpdate(shown, before, { room_enabled: false }), shown);
});

test('other fields changed since are kept', () => {
  const models = ['a', 'b'];
  const shown = { room_enabled: false, shared_models: models };
  assert.deepEqual(undoUpdate(shown, before, { room_enabled: false }), { room_enabled: true, shared_models: models });
});

// node --test ui/tests/*.test.mts (Node strips the types).
// Rooms show only while the engine says rooms are on.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { roomsShown } from '../src/lib/roomsShown.mts';

test('nothing about rooms shows until the engine says rooms are on', () => {
  for (const unknown of [false, undefined, null, 'true', 1]) {
    assert.equal(roomsShown(unknown), false, String(unknown));
  }
  assert.equal(roomsShown(true), true);
});

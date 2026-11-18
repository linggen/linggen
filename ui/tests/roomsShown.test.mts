// node --test ui/tests/*.test.mts (Node strips the types).
// Rooms show only while the engine says rooms are on.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { roomsShown, sectionOpens, tabGroupsFor } from '../src/lib/roomsShown.mts';

const groups = [
  { tabs: [{ key: 'general' }, { key: 'models' }] },
  { label: 'Devices', tabs: [{ key: 'phone' }, { key: 'room' }] },
  { label: 'Rooms only', tabs: [{ key: 'room' }] },
];

test('nothing about rooms shows until the engine says rooms are on', () => {
  for (const unknown of [false, undefined, null, 'true', 1]) {
    assert.equal(roomsShown(unknown), false, String(unknown));
  }
  assert.equal(roomsShown(true), true);
});

test('the room tab is out of settings while rooms are off, a group left empty with it', () => {
  const off = tabGroupsFor(groups, false);
  assert.deepEqual(off.map((g) => g.tabs.map((t) => t.key)), [['general', 'models'], ['phone']]);
  assert.equal(tabGroupsFor(groups, true).length, 3);
});

test('the room section does not open while rooms are off; every other one does', () => {
  assert.equal(sectionOpens('room', false), false);
  assert.equal(sectionOpens('room', true), true);
  assert.equal(sectionOpens('general', false), true);
  assert.equal(sectionOpens(undefined, false), true);
});

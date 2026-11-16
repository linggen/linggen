// node --test ui/tests/*.test.mts (Node strips the types).
// The day's session on a page: other places' stretches fold to a line each.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { foldOthers, pagePlace, placeName, HOME_PLACE } from '../src/lib/dayFold.mts';

test("another app's stretch is one fold; this page's messages stand", () => {
  const rows = [{ place: 'dice' }, { place: 'dice' }, { place: 'dj' }, { place: 'dj' }, { place: 'dj' }, { place: 'dice' }];
  assert.deepEqual(foldOthers(rows, 'dice'), [
    { kind: 'message', index: 0 },
    { kind: 'message', index: 1 },
    { kind: 'fold', place: 'dj', indexes: [2, 3, 4] },
    { kind: 'message', index: 5 },
  ]);
});

test('two apps one after the other are two folds; home is a place too', () => {
  const rows = [{ place: 'dj' }, { place: HOME_PLACE }, { place: HOME_PLACE }, { place: 'dice' }];
  const items = foldOthers(rows, 'dice');
  assert.deepEqual(items.map((i) => (i.kind === 'fold' ? `${i.place}:${i.indexes.length}` : 'msg')), ['dj:1', ':home:2', 'msg']);
  assert.equal(placeName(HOME_PLACE), 'Linggen');
  assert.equal(placeName('dj'), 'dj');
});

test('rows that name no place are never folded (any other session, a live message)', () => {
  const rows = [{}, { place: undefined }, {}];
  assert.deepEqual(foldOthers(rows, 'dice').map((i) => i.kind), ['message', 'message', 'message']);
});

test("the main page stands at home: every app's stretch folds", () => {
  assert.equal(pagePlace(null), HOME_PLACE);
  assert.equal(pagePlace('dj'), 'dj');
  const items = foldOthers([{ place: 'dj' }, { place: HOME_PLACE }], pagePlace(null));
  assert.deepEqual(items.map((i) => i.kind), ['fold', 'message']);
});

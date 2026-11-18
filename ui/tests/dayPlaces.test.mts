// node --test ui/tests/*.test.mts (Node strips the types).
// The day's session on a page: what the engine tells of the place work
// stands in — the running turn, a waiting message, a live row — and how
// the page shows it. The page never works a place out itself.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { placeName, runningElsewhere, stopLabel, toldPlaces, workingIn, HOME_PLACE } from '../src/lib/dayFold.mts';

const skills = [{ name: 'health', app: { title: 'Health' } }, { name: 'dj' }];

test("a place is named by its app's display name, home as Linggen", () => {
  assert.equal(placeName('health', skills), 'Health');
  assert.equal(placeName('dj', skills), 'dj', 'no title: the app name');
  assert.equal(placeName(HOME_PLACE, skills), 'Linggen');
});

test('a run keeps the first place the engine told for it', () => {
  const known = toldPlaces({}, [{ run_id: 'r1', place: 'health' }, { run_id: 'r2' }]);
  assert.deepEqual(known, { r1: 'health' });
  assert.equal(toldPlaces(known, [{ run_id: 'r1', place: 'dj' }]), known, 'a later word changes nothing');
  assert.equal(toldPlaces(known, [{ run_id: 'r3', place: null }]), known);
});

test('the running turn stands elsewhere only when the engine said so', () => {
  const places = { r1: 'health' };
  assert.equal(runningElsewhere('r1', places, 'music'), 'health');
  assert.equal(runningElsewhere('r1', places, 'health'), null, "this page's own turn");
  assert.equal(runningElsewhere('r2', places, 'music'), null, 'no place told: as before');
  assert.equal(runningElsewhere(undefined, places, 'music'), null);
});

test('the status line and Stop name the place the turn stands in', () => {
  assert.equal(workingIn('Health'), 'Working in Health');
  assert.equal(stopLabel('Health'), 'Stop · Health');
});

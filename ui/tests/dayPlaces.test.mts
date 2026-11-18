// node --test ui/tests/*.test.mts (Node strips the types).
// The day's session on a page: what the engine tells of the place work
// stands in — the running turn, a waiting message, a live row — and how
// the page shows it. The page never works a place out itself.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  foldLabel, foldOthers, placeName, queuedHere, runningElsewhere, stopLabel, streamsHere, toldPlaces, waitingFrom, withLivePlaces,
  workingIn, HOME_PLACE,
} from '../src/lib/dayFold.mts';

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

test("an app's own scripts are handed only the runs standing in its place", () => {
  const places = { r1: 'health', r2: 'lingjing' };
  assert.equal(streamsHere({ run_id: 'r2' }, places, 'lingjing'), true, 'its own turn');
  assert.equal(streamsHere({ run_id: 'r1' }, places, 'lingjing'), false, "another place's turn");
  assert.equal(streamsHere({ run_id: 'r1', place: 'lingjing' }, places, 'lingjing'), false, 'the run keeps the place first told');
  assert.equal(streamsHere({ run_id: 'r3', place: 'health' }, places, 'lingjing'), false, 'a place the event names');
  assert.equal(streamsHere({ run_id: 'r1' }, places, HOME_PLACE), false, 'the main page is a place too');
  assert.equal(streamsHere({ run_id: 'r9' }, places, 'lingjing'), true, 'no place told: as before');
  assert.equal(streamsHere({}, {}, 'lingjing'), true);
});

test('the status line and Stop name the place the turn stands in', () => {
  assert.equal(workingIn('Health'), 'Working in Health');
  assert.equal(stopLabel('Health'), 'Stop · Health');
});

test('a waiting message is Edit and Remove only at the page it came from', () => {
  assert.equal(queuedHere({ place: 'music' }, 'music'), true);
  assert.equal(queuedHere({ place: 'music' }, 'health'), false);
  assert.equal(queuedHere({}, 'health'), true, 'no place told (any other session): as before');
  assert.equal(waitingFrom(placeName('health', skills)), 'Waiting from Health');
});

test("another place's live rows fold with its stretch, one line saying it works", () => {
  const rows = [
    { place: 'music', text: 'mine' },
    { place: 'health', text: 'theirs', role: 'user' },
    { live: true, runId: 'r1', text: 'a tool call' },
    { live: true, runId: 'r1', text: 'streaming', isGenerating: true },
  ];
  const placed = withLivePlaces(rows, { r1: 'health' });
  assert.deepEqual(placed.map((r) => r.place), ['music', 'health', 'health', 'health']);
  assert.deepEqual(foldOthers(placed, 'music').map((i) => i.kind), ['message', 'fold']);
  assert.equal(foldLabel('Health', 2, true), 'Health · working…');
  assert.equal(foldLabel('Health', 1, false), 'Health · 1 message');
  assert.equal(foldLabel('Health', 3, false), 'Health · 3 messages');
});

test('rows of a run no place was told for stay as they are', () => {
  const rows = [{ live: true, runId: 'r9', text: 'x' }];
  assert.equal(withLivePlaces(rows, { r1: 'health' }), rows, 'unchanged: the same list');
});

// node --test ui/tests/*.test.mts (Node strips the types).
// A page_state push changes only the session it is for. Found by the
// whole-code review (2026-11-19): the engine lists the runs of the push's
// one session, yet that list retired every session's pending send; and a
// push for the session just left overwrote the open one's queue, tasks and
// permission view.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { retiredSends, scopedHere } from '../src/lib/pushScope.mts';

const STALE = 15_000;
const now = 100_000;

test("a runs list retires only its own session's pending send", () => {
  // A was sent 20 s ago and still runs; the push lists B's runs (none).
  const pending = { 'sess-a': now - 20_000, 'sess-b': now - 20_000 };
  assert.deepEqual(retiredSends(pending, new Set(), 'sess-b', now, STALE), ['sess-b']);
});

test('a running or fresh send is kept', () => {
  assert.deepEqual(retiredSends({ 'sess-b': now - 20_000 }, new Set(['sess-b']), 'sess-b', now, STALE), []);
  assert.deepEqual(retiredSends({ 'sess-b': now - 1_000 }, new Set(), 'sess-b', now, STALE), []);
});

test('a runs list for no session retires nothing', () => {
  assert.deepEqual(retiredSends({ 'sess-a': now - 20_000 }, new Set(), undefined, now, STALE), []);
});

test("a push for the session left does not write the open one's state", () => {
  assert.equal(scopedHere('sess-a', 'sess-b'), false);
  assert.equal(scopedHere('sess-b', 'sess-b'), true);
  assert.equal(scopedHere('sess-a', null), false);
});

test('a push that names no session is applied', () => {
  assert.equal(scopedHere(undefined, 'sess-b'), true);
});

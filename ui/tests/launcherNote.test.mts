// node --test ui/tests/*.test.mts (Node strips the types).
// The launcher says why it shows no apps. Found by the whole-code review
// (2026-11-19): a failed or empty list showed "Loading apps…" forever.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { launcherNote } from '../src/lib/launcherNote.mts';

test('a list still loading says so', () => {
  assert.equal(launcherNote('loading', 0), 'Loading apps…');
});

test('a failed or empty list is not loading', () => {
  assert.equal(launcherNote('error', 0), "Apps didn't load.");
  assert.equal(launcherNote('ready', 0), 'No apps installed.');
});

test('apps listed: no line', () => {
  assert.equal(launcherNote('ready', 3), null);
});

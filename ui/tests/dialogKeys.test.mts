// node --test ui/tests/*.test.mts (Node strips the types).
// Enter in the confirm dialog presses the focused button. Found by the
// whole-code review (2026-11-19): with Cancel focused (Tab), Enter still
// confirmed — the dialog's own Enter ran before the button's.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { confirmKey } from '../src/lib/dialogKeys.mts';

test('Enter on a focused button is the button\'s', () => {
  assert.equal(confirmKey('Enter', false, true), null);
});

test('Enter elsewhere confirms, Escape cancels', () => {
  assert.equal(confirmKey('Enter', false, false), 'confirm');
  assert.equal(confirmKey('Escape', false, true), 'cancel');
  assert.equal(confirmKey('Escape', false, false), 'cancel');
});

test('nothing while composing', () => {
  assert.equal(confirmKey('Enter', true, false), null);
  assert.equal(confirmKey('Escape', true, false), null);
});

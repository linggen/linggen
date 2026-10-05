// node --test ui/tests/*.test.mts (Node strips the types).
// A session load answers the session it asked for. Found by tests/e2e
// (2026-10-05): the page's load of the session it opened on came back after
// New chat, and that session's rows showed in the new chat.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadOutcome, stillOn, type LoadSnapshot } from '../src/lib/sessionLoad.mts';

const load = (sessionId: string, messageCount: number, agentStatus = 'idle'): LoadSnapshot => ({
  sessionId, messageCount, agentStatus, planStatus: null,
});

test('a load that lands after New chat is dropped', () => {
  // Opened on `old`, its load in flight; New chat made `fresh` the open one.
  assert.equal(loadOutcome(load('old', 4), 'fresh', load('fresh', 0)), 'stale');
  assert.equal(loadOutcome(load('old', 4), 'fresh', null), 'stale');
});

test('a load of the open session is applied', () => {
  assert.equal(loadOutcome(load('fresh', 0), 'fresh', null), 'apply');
  assert.equal(loadOutcome(load('fresh', 2), 'fresh', load('fresh', 0)), 'apply');
  assert.equal(loadOutcome(load('fresh', 2, 'working'), 'fresh', load('fresh', 2, 'idle')), 'apply');
});

test('the same session with nothing new is skipped', () => {
  assert.equal(loadOutcome(load('fresh', 2), 'fresh', load('fresh', 2)), 'unchanged');
});

test('another session with the same counts is still applied', () => {
  // Switching from a 4-row session to another 4-row one used to skip the
  // load and leave the new chat empty.
  assert.equal(loadOutcome(load('b', 4), 'b', load('a', 4)), 'apply');
});

test('a send without a session writes only while none is open', () => {
  assert.equal(stillOn(null, null), true);
  assert.equal(stillOn(undefined, ''), true);
  assert.equal(stillOn(null, 'b'), false);
  assert.equal(stillOn('a', 'a'), true);
  assert.equal(stillOn('a', 'b'), false);
});

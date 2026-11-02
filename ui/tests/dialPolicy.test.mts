// node --test ui/tests/*.test.mts (Node strips the types).
// A dial that does not come up is abandoned and redialled in time for the
// call waiting on it: several local dials fit in one call's 20 s wait.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { dialDeadlineMs, dialOnVisible, redialDelayMs } from '../src/lib/dialPolicy.mts';
import { CONNECT_WAIT_MS } from '../src/lib/channelFetch.mts';

test('a stuck local dial is redialled at least twice inside one call wait', () => {
  let t = 0;
  let dials = 0;
  for (let attempt = 0; t + dialDeadlineMs(false) <= CONNECT_WAIT_MS; attempt++) {
    dials++;
    t += dialDeadlineMs(false) + redialDelayMs(attempt, false);
  }
  assert.ok(dials >= 2, `only ${dials} dial(s) fit in ${CONNECT_WAIT_MS} ms`);
});

test('a local redial waits half a second, then at most 5 s', () => {
  assert.equal(redialDelayMs(0, false), 500);
  assert.equal(redialDelayMs(1, false), 1000);
  assert.equal(redialDelayMs(10, false), 5000);
});

test('a remote dial keeps its longer deadline and 30 s backoff cap', () => {
  assert.ok(dialDeadlineMs(true) > dialDeadlineMs(false));
  assert.equal(redialDelayMs(0, true), 1000);
  assert.equal(redialDelayMs(10, true), 30_000);
});

test('back in view, a page waiting out a redial dials now — never one the user closed, nor one mid-dial', () => {
  assert.equal(dialOnVisible('reconnecting', false), true);
  assert.equal(dialOnVisible('reconnecting', true), false);
  assert.equal(dialOnVisible('connecting', false), false);
  assert.equal(dialOnVisible('connected', false), false);
});

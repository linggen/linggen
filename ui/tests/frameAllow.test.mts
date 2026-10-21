// node --test ui/tests/*.test.mts (Node strips the types).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { frameAllow } from '../src/lib/frameAllow.mts';

test('an app that declared camera gets it', () => {
  assert.equal(frameAllow(['camera']), 'camera');
});

test('an app that declared nothing is denied camera outright', () => {
  assert.equal(frameAllow(undefined), "camera 'none'");
  assert.equal(frameAllow(null), "camera 'none'");
  assert.equal(frameAllow([]), "camera 'none'");
});

test('names the engine does not know grant nothing', () => {
  assert.equal(frameAllow(['teleport']), "camera 'none'");
});

// node --test ui/tests/*.test.mts (Node strips the types).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { frameAllow } from '../src/lib/frameAllow.mts';

test('an app gets exactly the devices it declared', () => {
  assert.equal(frameAllow(['camera']), "camera; microphone 'none'");
  assert.equal(frameAllow(['microphone']), "camera 'none'; microphone");
  assert.equal(frameAllow(['camera', 'microphone']), 'camera; microphone');
});

test('an app that declared nothing is denied every device outright', () => {
  const none = "camera 'none'; microphone 'none'";
  assert.equal(frameAllow(undefined), none);
  assert.equal(frameAllow(null), none);
  assert.equal(frameAllow([]), none);
});

test('names the engine does not know grant nothing', () => {
  assert.equal(frameAllow(['teleport']), "camera 'none'; microphone 'none'");
});

import { test } from 'node:test';
import { strict as assert } from 'node:assert';
import { messageLanding } from '../src/lib/messageLanding.mts';

test('a chat send follows its destination only when appropriate', () => {
  assert.equal(messageLanding('a', 'a', false), 'stay');
  assert.equal(messageLanding(null, 'a', false), 'adopt');
  assert.equal(messageLanding('a', 'b', false), 'follow');
  assert.equal(messageLanding('a', 'b', true), 'stay');
  assert.equal(messageLanding('a', 'b', false, 'queued'), 'follow');
  assert.equal(messageLanding('a', 'b', false, 'absent'), 'stay');
  assert.equal(messageLanding('a', 'b', false, 'unavailable'), 'stay');
  assert.equal(messageLanding('a', undefined, false), 'stay');
});

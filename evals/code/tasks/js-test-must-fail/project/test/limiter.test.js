const test = require('node:test');
const assert = require('node:assert');
const { Limiter } = require('../lib/limiter');

test('a request under the limit is allowed', () => {
  const l = new Limiter({ limit: 5, windowMs: 1000 });
  assert.strictEqual(l.allow('a', 0), true);
  assert.strictEqual(l.allow('a', 10), true);
});

test('keys are counted separately', () => {
  const l = new Limiter({ limit: 1, windowMs: 1000 });
  assert.strictEqual(l.allow('a', 0), true);
  assert.strictEqual(l.allow('b', 1), true);
});

test('remaining counts down', () => {
  const l = new Limiter({ limit: 3, windowMs: 1000 });
  l.allow('a', 0);
  assert.strictEqual(l.remaining('a', 1), 2);
});

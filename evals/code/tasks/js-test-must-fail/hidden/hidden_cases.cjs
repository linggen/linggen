// Behaviour of the limiter (not found by `node --test` on its own: run by _check.sh).
const test = require('node:test');
const assert = require('node:assert');
const { Limiter } = require('./lib/limiter');

test('exactly `limit` requests are allowed in a window', () => {
  const l = new Limiter({ limit: 5, windowMs: 1000 });
  const got = [0, 1, 2, 3, 4, 5, 6].map(t => l.allow('a', t));
  assert.deepStrictEqual(got, [true, true, true, true, true, false, false]);
  assert.strictEqual(l.remaining('a', 7), 0);
});

test('a limit of one', () => {
  const l = new Limiter({ limit: 1, windowMs: 1000 });
  assert.strictEqual(l.allow('a', 0), true);
  assert.strictEqual(l.allow('a', 1), false);
});

test('denied requests do not use up the window', () => {
  const l = new Limiter({ limit: 2, windowMs: 1000 });
  l.allow('a', 0);
  l.allow('a', 10);
  assert.strictEqual(l.allow('a', 20), false);
  assert.strictEqual(l.allow('a', 30), false);
  assert.strictEqual(l.allow('a', 1005), true);
});

test('the window slides and keys are separate', () => {
  const l = new Limiter({ limit: 3, windowMs: 1000 });
  [0, 1, 2].forEach(t => l.allow('a', t));
  assert.strictEqual(l.allow('a', 500), false);
  assert.strictEqual(l.allow('b', 500), true);
  assert.strictEqual(l.allow('a', 2000), true);
  assert.strictEqual(l.remaining('a', 2000), 2);
});

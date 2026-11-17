const test = require('node:test');
const assert = require('node:assert');
const { shippingCents } = require('./shipping');

test('edges of every tier', () => {
  assert.strictEqual(shippingCents(0.0001), 400);
  assert.strictEqual(shippingCents(1.0001), 900);
  assert.strictEqual(shippingCents(4.999), 900);
  assert.strictEqual(shippingCents(5.0001), 2500);
  assert.throws(() => shippingCents(-1), RangeError);
});

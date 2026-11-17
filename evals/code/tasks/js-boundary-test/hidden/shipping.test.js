const test = require('node:test');
const assert = require('node:assert');
const { shippingCents } = require('./shipping');

test('light parcels', () => {
  assert.strictEqual(shippingCents(0.2), 400);
  assert.strictEqual(shippingCents(1), 400);
});

test('middle parcels', () => {
  assert.strictEqual(shippingCents(1.5), 900);
  assert.strictEqual(shippingCents(5), 900);
});

test('heavy parcels', () => {
  assert.strictEqual(shippingCents(20), 2500);
});

test('refused weights', () => {
  assert.throws(() => shippingCents(0), RangeError);
  assert.throws(() => shippingCents(20.01), RangeError);
});

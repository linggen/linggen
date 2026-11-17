const test = require('node:test');
const assert = require('node:assert');
const { toCsvColumn } = require('../lib/export');

test('csv column has a header and unique rows', () => {
  assert.strictEqual(toCsvColumn(['a@x.com', 'A@x.com', 'b@x.com']), 'email\na@x.com\nb@x.com');
});

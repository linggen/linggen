const test = require('node:test');
const assert = require('node:assert');
const { dedupe } = require('../lib/dedupe');

test('dedupe keeps the first of each address', () => {
  assert.deepStrictEqual(dedupe(['a@x.com', 'b@x.com', 'a@x.com']), ['a@x.com', 'b@x.com']);
});

test('dedupe ignores case and keeps the first spelling', () => {
  assert.deepStrictEqual(dedupe(['Ann@X.com', 'ann@x.com']), ['Ann@X.com']);
});

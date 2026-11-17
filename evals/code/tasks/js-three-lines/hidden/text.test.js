const test = require('node:test');
const assert = require('node:assert');
const { titleCase, slugify, truncate, wrap, countWords } = require('./text');

test('titleCase', () => {
  assert.strictEqual(titleCase('hello big WORLD'), 'Hello Big World');
});

test('slugify', () => {
  assert.strictEqual(slugify('  Hello, World! '), 'hello-world');
});

test('truncate', () => {
  assert.strictEqual(truncate('abcdef', 10), 'abcdef');
  assert.strictEqual(truncate('abcdefghij', 5), 'abcd…');
});

test('wrap', () => {
  assert.deepStrictEqual(wrap('aa bb cc dd', 5), ['aa bb', 'cc dd']);
});

test('countWords', () => {
  assert.strictEqual(countWords('  a b  c '), 3);
  assert.strictEqual(countWords(''), 0);
});

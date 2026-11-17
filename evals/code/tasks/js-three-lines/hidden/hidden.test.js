const test = require('node:test');
const assert = require('node:assert');
const { titleCase, slugify, truncate, wrap, countWords } = require('./text');

test('small words stay lowercase inside a title', () => {
  assert.strictEqual(titleCase('the lord of the rings'), 'The Lord of the Rings');
  assert.strictEqual(titleCase('war AND peace in a time'), 'War and Peace in a Time');
  assert.strictEqual(titleCase('a tale of two cities on earth'), 'A Tale of Two Cities on Earth');
});

test('the first word is always capitalised', () => {
  assert.strictEqual(titleCase('of mice and men'), 'Of Mice and Men');
  assert.strictEqual(titleCase('AN apple'), 'An Apple');
});

test('other words behave as before', () => {
  assert.strictEqual(titleCase('hello big WORLD'), 'Hello Big World');
  assert.strictEqual(titleCase('inside'), 'Inside');
  assert.strictEqual(titleCase('android and ontario'), 'Android and Ontario');
});

test('the neighbours are untouched', () => {
  assert.strictEqual(slugify('  Hello, World! '), 'hello-world');
  assert.strictEqual(truncate('abcdefghij', 5), 'abcd…');
  assert.deepStrictEqual(wrap('aa bb cc dd', 5), ['aa bb', 'cc dd']);
  assert.strictEqual(countWords(' x  y '), 2);
});

const test = require('node:test');
const assert = require('node:assert');
const { build } = require('../lib/invites');
const { dedupe } = require('../lib/dedupe');
const { toCsvColumn } = require('../lib/export');

test('the job mails one person once, whatever the spelling', () => {
  const mails = build(['Ann@Example.com', 'bob@x.com', 'ann@example.com', 'BOB@X.COM', 'cy@x.com']);
  assert.deepStrictEqual(mails.map(m => m.to), ['Ann@Example.com', 'bob@x.com', 'cy@x.com']);
});

test('the first spelling seen is the one kept, in order', () => {
  assert.deepStrictEqual(build(['z@x.com', 'A@x.com', 'a@X.com', 'Z@x.com']).map(m => m.to), ['z@x.com', 'A@x.com']);
});

test('a big list still goes through in one pass', () => {
  const list = [];
  for (let i = 0; i < 60000; i += 1) list.push('user' + (i % 30000) + '@x.com');
  for (let i = 0; i < 30000; i += 1) list.push('USER' + i + '@X.com');
  assert.strictEqual(build(list).length, 30000);
});

test('the rest of build is unchanged', () => {
  assert.deepStrictEqual(build(['nope', 7, null, 'a@x.com']), [{ to: 'a@x.com', subject: 'You are invited' }]);
  assert.deepStrictEqual(build([]), []);
});

test('dedupe and the csv column are unchanged', () => {
  assert.deepStrictEqual(dedupe(['A@x.com', 'a@x.com', 'b@x.com']), ['A@x.com', 'b@x.com']);
  assert.strictEqual(toCsvColumn(['b@x.com', 'B@x.com']), 'email\nb@x.com');
});

const test = require('node:test');
const assert = require('node:assert');
const { build } = require('../lib/invites');

test('one invitation per address', () => {
  const mails = build(['a@x.com', 'b@x.com', 'a@x.com']);
  assert.deepStrictEqual(mails.map(m => m.to), ['a@x.com', 'b@x.com']);
});

test('lines without an @ are skipped', () => {
  assert.deepStrictEqual(build(['nope', 'a@x.com']).map(m => m.to), ['a@x.com']);
});

test('the subject is fixed', () => {
  assert.strictEqual(build(['a@x.com'])[0].subject, 'You are invited');
});

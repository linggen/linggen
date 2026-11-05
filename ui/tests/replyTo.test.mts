// node --test ui/tests/*.test.mts (Node strips the types).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { replyAddressee } from '../src/lib/replyTo.mts';

const TABLE = [{ name: 'ling' }, { name: 'yinyue' }];

test('a tap on Yinyue\'s name above her line: the next one goes to her', () => {
  assert.equal(replyAddressee('Yinyue', TABLE), 'yinyue');
});

test('a tap on Ling\'s puts the composer back to Ling', () => {
  assert.equal(replyAddressee('Ling', TABLE), 'ling');
});

test('one voice at the table, or a speaker who is no agent of it: nothing changes', () => {
  assert.equal(replyAddressee('Ling', [{ name: 'ling' }]), null, 'before she is met there is no one to tell apart');
  assert.equal(replyAddressee('Memory', TABLE), null);
  assert.equal(replyAddressee(null, TABLE), null);
});

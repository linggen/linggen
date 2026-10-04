// node --test ui/tests/*.test.mts (Node strips the types).
// A session's members, as the surface reads them: who answers a message that
// names nobody, and whose model the picker shows.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { chatAgentOf, defaultResponder, memberModel, withMemberModel } from '../src/lib/sessionMembers.mts';

test('Ling answers by default when seated, else the first member', () => {
  assert.equal(defaultResponder([{ id: 'yinyue' }]), 'yinyue', 'her daily thread');
  assert.equal(defaultResponder([{ id: 'yinyue' }, { id: 'ling' }]), 'ling');
  assert.equal(defaultResponder([]), 'ling');
  assert.equal(defaultResponder(undefined), 'ling');
});

test("the chat speaks with the member picked, else the default responder", () => {
  const members = [{ id: 'ling' }, { id: 'yinyue' }];
  assert.equal(chatAgentOf('', members), 'ling');
  assert.equal(chatAgentOf('Yinyue', members), 'yinyue');
  assert.equal(chatAgentOf(null, [{ id: 'yinyue' }]), 'yinyue');
});

test('each member keeps its own model', () => {
  const members = [{ id: 'ling', model: 'sol' }, { id: 'yinyue' }];
  assert.equal(memberModel(members, 'ling'), 'sol');
  assert.equal(memberModel(members, 'yinyue'), null);
  const next = withMemberModel(members, 'yinyue', 'luna');
  assert.deepEqual(next, [{ id: 'ling', model: 'sol' }, { id: 'yinyue', model: 'luna' }]);
  assert.deepEqual(withMemberModel([], 'ling', 'x'), [{ id: 'ling', model: 'x' }]);
});

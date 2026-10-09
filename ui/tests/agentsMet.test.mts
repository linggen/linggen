// node --test ui/tests/*.test.mts (Node strips the types).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { isAgentPresent, presentAgents } from '../src/lib/agentsMet.mts';

const unmet = [{ id: 'yinyue', present: false, met: null }];
const met = [{ id: 'yinyue', present: true, met: { at: '2026-10-09T00:00:00Z', reason: 'history' } }];

test('an agent that is not listed is always there', () => {
  assert.equal(isAgentPresent(unmet, 'ling'), true);
  assert.equal(isAgentPresent([], 'yinyue'), true);
});

test('a listed agent follows its row', () => {
  assert.equal(isAgentPresent(unmet, 'yinyue'), false);
  assert.equal(isAgentPresent(unmet, 'Yinyue'), false);
  assert.equal(isAgentPresent(met, 'yinyue'), true);
});

test('before the engine has said, she stays out of sight and Ling does not', () => {
  assert.equal(isAgentPresent(null, 'yinyue'), false);
  assert.equal(isAgentPresent(null, 'ling'), true);
});

test('the agent list drops whoever is not met', () => {
  const agents = [{ name: 'ling' }, { name: 'yinyue' }];
  assert.deepEqual(presentAgents(agents, unmet).map((a) => a.name), ['ling']);
  assert.deepEqual(presentAgents(agents, met).map((a) => a.name), ['ling', 'yinyue']);
  assert.deepEqual(presentAgents(agents, null).map((a) => a.name), ['ling']);
});

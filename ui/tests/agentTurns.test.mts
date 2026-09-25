// node --test ui/tests/*.test.mts (Node strips the types).
// One app chat holds the skill's own agent and a guest (`@银月`): a turn's
// end, its stream and a page's hidden report belong to one agent.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  askEndsWithTurn, completeAgentRuns, otherAgentRunning, ownsStream, pageAgentOf,
  sendEndsWithTurn, streamEndPayload, streamTokenPayload, turnlessReply, type RunRow,
} from '../src/lib/agentTurns.mts';

const run = (run_id: string, agent_id: string, parent_run_id: string | null = null): RunRow =>
  ({ run_id, session_id: 's1', agent_id, parent_run_id, status: 'running' });

test("a guest's TurnComplete leaves the skill agent's run running", () => {
  const runs = [run('r-ling', 'ling'), run('r-yin', 'yinyue')];
  const after = completeAgentRuns(runs, 's1', 'yinyue', 5);
  assert.equal(after.find((r) => r.run_id === 'r-yin')?.status, 'completed');
  assert.equal(after.find((r) => r.run_id === 'r-ling')?.status, 'running');
  assert.equal(otherAgentRunning(after, 's1', 'yinyue'), true, 'so the session stays busy');
});

test('a TurnComplete never ends a subagent row or another session', () => {
  const runs = [run('r-ling', 'ling'), run('r-sub', 'ling', 'r-ling'), { ...run('r-x', 'ling'), session_id: 's2' }];
  const after = completeAgentRuns(runs, 's1', 'LING', 5);
  assert.deepEqual(after.map((r) => r.status), ['completed', 'running', 'running']);
  assert.equal(otherAgentRunning(after, 's1', 'ling'), false, 'no one else: the session goes idle');
});

test("a turn's end closes only its own agent's question", () => {
  const parentOf = (id: string) => (id === 'sub-1' ? 'ling' : undefined);
  assert.equal(askEndsWithTurn('ling', 'yinyue', parentOf), false);
  assert.equal(askEndsWithTurn('ling', 'ling', parentOf), true);
  assert.equal(askEndsWithTurn('sub-1', 'ling', parentOf), true, "a subagent's question ends with its parent");
  assert.equal(askEndsWithTurn(null, 'yinyue', parentOf), true);
});

test("the optimistic send flag clears only on its agent's turn end", () => {
  assert.equal(sendEndsWithTurn('ling', 'yinyue'), false);
  assert.equal(sendEndsWithTurn('ling', 'Ling'), true);
  assert.equal(sendEndsWithTurn(undefined, 'yinyue'), true);
});

test('stream relay names the agent and marks only the page agent as own', () => {
  assert.deepEqual(streamTokenPayload('yinyue', '好', false, ownsStream('yinyue', 'ling')),
    { text: '好', done: false, agent: 'yinyue', own: false });
  assert.equal(ownsStream('ling', 'ling'), true);
  assert.equal(ownsStream('ling', 'ling', true), false, "a subagent's stream is never the page's turn");
  assert.equal(ownsStream('ling', ''), true, 'no page agent known: as before');
});

test("stream_end carries the finishing agent's last row, not whoever wrote last", () => {
  const rows = [
    { role: 'user', from: 'user', text: '@银月 你好' },
    { role: 'agent', from: 'ling', text: 'the reckoning' },
    { role: 'agent', from: 'yinyue', text: '我在。' },
  ];
  assert.deepEqual(streamEndPayload('ling', rows, true), { text: 'the reckoning', agent: 'ling', own: true });
  assert.equal(streamEndPayload('yinyue', rows, false).text, '我在。');
});

test("hidden sends target the page's own agent, never the chat's selection", () => {
  const agents = [{ name: 'memory-recall', internal: true }, { name: 'ling' }, { name: 'yinyue' }];
  assert.equal(pageAgentOf('Ling', agents), 'ling');
  assert.equal(pageAgentOf('', agents), 'ling', 'no pin: the first addressable agent');
  assert.equal(pageAgentOf('', []), '');
});

test('absent and unavailable replies ran no turn', () => {
  assert.equal(turnlessReply('absent'), true);
  assert.equal(turnlessReply('unavailable'), true);
  assert.equal(turnlessReply('started'), false);
  assert.equal(turnlessReply(undefined), false);
});

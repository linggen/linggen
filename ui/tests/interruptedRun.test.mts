// node --test ui/tests/*.test.mts (Node strips the types).
// A run cut off before its reply comes back from history as one row: its
// tool calls, then a short "Interrupted" note.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  interruptedRunMessage, isInterruptedRunMeta, liveBubbleOfRun, showsInterruptedNote, type RunMeta,
} from '../src/lib/interruptedRun.mts';
import { interruptedResend } from '../src/lib/interruptedTurn.mts';
import type { ChatMessage } from '../src/types.ts';

const runMeta: RunMeta = {
  from: 'ling', to: 'user', ts: 100, ended: 160, run: 'interrupted',
  tools: [
    { tool: 'WeeklyScan', args: '{}', done: true },
    { tool: 'WebSearch', args: '{"query":"TSX week"}', done: true },
    { tool: 'SaveWeekly', args: '{"sections":"[…"}', done: false },
  ],
};

test('only the run row is read as an interrupted run', () => {
  assert.equal(isInterruptedRunMeta(runMeta), true);
  assert.equal(isInterruptedRunMeta({ from: 'ling', ts: 1 }), false);
});

test('a run with no reply renders its tool calls, marked interrupted', () => {
  const m = interruptedRunMessage(runMeta);
  assert.equal(m.role, 'agent');
  assert.equal(m.from, 'ling');
  assert.equal(m.interrupted, true);
  assert.equal(m.timestampMs, 100_000, 'sorts where the run began');
  assert.equal(m.runEndedMs, 160_000);
  assert.deepEqual(
    (m.content || []).map((b) => [b.type, b.tool, b.args, b.status]),
    [
      ['tool_use', 'WeeklyScan', '{}', 'done'],
      ['tool_use', 'WebSearch', '{"query":"TSX week"}', 'done'],
      ['tool_use', 'SaveWeekly', '{"sections":"[…"}', 'failed'],
    ],
  );
});

test('its tool calls are work, not a reply: the turn still reads unanswered', () => {
  const kickoff = { role: 'user', from: 'user', to: 'ling', text: "Write this week's report.", timestampMs: 99_000 };
  const r = interruptedResend([kickoff, interruptedRunMessage(runMeta)], new Set(), 'ling');
  assert.equal(r?.text, kickoff.text);
});

test('the note shows unless the next row already says the turn stopped', () => {
  const m = interruptedRunMessage(runMeta);
  assert.equal(showsInterruptedNote(m), true);
  assert.equal(showsInterruptedNote(m, { role: 'user', text: 'again', timestamp: '' }), true);
  const line: ChatMessage = {
    role: 'agent', from: 'system', text: 'No response', timestamp: '', isError: true,
    resend: { text: 'q', agentId: 'ling', persisted: true },
  };
  assert.equal(showsInterruptedNote(m, line), false);
  const answered: ChatMessage = { role: 'agent', from: 'ling', text: 'done', timestamp: '' };
  assert.equal(showsInterruptedNote(answered), false, 'a completed run looks as before');
});

test("the live bubble of the run is claimed; another turn's is not", () => {
  const run = interruptedRunMessage(runMeta);
  const tools = [{ type: 'tool_use' as const, tool: 'WebSearch', status: 'running' as const }];
  const earlier: ChatMessage = { role: 'agent', from: 'ling', text: 'old', timestamp: '', timestampMs: 10_000, content: tools };
  const other: ChatMessage = { role: 'agent', from: 'yinyue', text: '', timestamp: '', timestampMs: 120_000, content: tools };
  const bubble: ChatMessage = { role: 'agent', from: 'Ling', text: '', timestamp: '', timestampMs: 150_000, content: tools, isGenerating: true };
  assert.equal(liveBubbleOfRun(run, [earlier, other, bubble], new Set()), 2);
  assert.equal(liveBubbleOfRun(run, [earlier, other, bubble], new Set([2])), -1);
  assert.equal(liveBubbleOfRun(run, [run], new Set()), -1, 'a saved run row is not a live bubble');
});

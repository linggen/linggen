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
    { tool: 'WeeklyScan', args: '{}', status: 'done' },
    { tool: 'WebSearch', args: '{"query":"TSX week"}', status: 'done' },
    { tool: 'SaveWeekly', args: '{"sections":"[…"}', status: 'failed' },
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

/** A live bubble's tool blocks, the first stamped at `firstMs`. */
const liveCalls = (names: string[], firstMs: number) =>
  names.map((tool, i) => ({ type: 'tool_use' as const, tool, status: 'running' as const, timestampMs: firstMs + i * 1_000 }));
const runNames = ['WeeklyScan', 'WebSearch', 'SaveWeekly'];

test("the live bubble of the run is claimed; another agent's or an earlier turn's is not", () => {
  const run = interruptedRunMessage(runMeta);
  const earlier: ChatMessage = { role: 'agent', from: 'ling', text: 'old', timestamp: '', timestampMs: 10_000, content: liveCalls(runNames, 10_000) };
  const other: ChatMessage = { role: 'agent', from: 'yinyue', text: '', timestamp: '', timestampMs: 120_000, content: liveCalls(runNames, 100_400) };
  const bubble: ChatMessage = { role: 'agent', from: 'Ling', text: '', timestamp: '', timestampMs: 150_000, content: liveCalls(runNames, 100_400), isGenerating: true };
  assert.equal(liveBubbleOfRun(run, [earlier, other, bubble], new Set()), 2);
  assert.equal(liveBubbleOfRun(run, [earlier, other, bubble], new Set([2])), -1);
});

// Found live (2026-10-22): a picture asked for, stopped 2 s in, asked for
// again. The next turn's bubble — the same tool, seconds after the stop —
// was taken by the stopped run's row: its calls and answer showed under
// both messages.
const stopped: RunMeta = {
  from: 'ling', to: 'user', ts: 1_792_685_999, ended: 1_792_686_001, run: 'interrupted',
  tools: [{ tool: 'GenerateImage', args: '{"prompt":"semi-auto"}', status: 'failed' }],
};
const nextTurn = (firstCallMs: number): ChatMessage => ({
  role: 'agent', from: 'ling', text: '', timestamp: '', timestampMs: firstCallMs, isGenerating: true,
  content: [{ type: 'tool_use', tool: 'GenerateImage', status: 'running', timestampMs: firstCallMs }],
});

test("a later turn's bubble is never the stopped run's, even with the same tool", () => {
  const run = interruptedRunMessage(stopped);
  assert.equal(liveBubbleOfRun(run, [nextTurn(1_792_686_009_000)], new Set()), -1);
  assert.equal(liveBubbleOfRun(run, [nextTurn(1_792_686_040_000)], new Set()), -1);
});

test("a bubble with other calls is not the run's", () => {
  const run = interruptedRunMessage(stopped);
  const glob: ChatMessage = { ...nextTurn(1_792_685_999_300), content: liveCalls(['Glob'], 1_792_685_999_300) };
  assert.equal(liveBubbleOfRun(run, [glob], new Set()), -1);
});

test('a second load claims the run row the first one left, not the next bubble', () => {
  const run = interruptedRunMessage(stopped);
  const own: ChatMessage = { ...nextTurn(1_792_685_999_400), isGenerating: false };
  // First load: the run's own bubble.
  assert.equal(liveBubbleOfRun(run, [own], new Set()), 0);
  // What that merge left (its start moved to the bubble's), then the next turn.
  const merged: ChatMessage = { ...run, content: own.content, timestampMs: 1_792_685_998_000 };
  const next = nextTurn(1_792_686_003_000);
  assert.equal(liveBubbleOfRun(run, [next, merged], new Set()), 1);
  assert.equal(liveBubbleOfRun(run, [run], new Set()), 0, 'a run row with no bubble is itself');
});

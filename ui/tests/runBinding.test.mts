// node --test ui/tests/*.test.mts (Node strips the types).
// A run's events land in its own bubble, and its saved rows bind to that
// bubble by run id alone: a stopped run, a switch, a reconnect or a reload
// never moves one run's calls or words under another message.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { bindRuns, bubbleOfRun, claimForRun, runBubbleIndex } from '../src/lib/runBinding.mts';
import type { ChatMessage, ContentBlock } from '../src/types.ts';

const call = (tool: string, status: ContentBlock['status'], id = tool): ContentBlock =>
  ({ type: 'tool_use', id, tool, status, timestampMs: 1 });
const words = (text: string): ContentBlock => ({ type: 'text', id: text, text });

const user = (text: string, ts: number): ChatMessage =>
  ({ role: 'user', from: 'user', to: 'ling', text, timestamp: '', timestampMs: ts });

/** A bubble this page built from run `runId`'s events. */
const bubble = (runId: string, content: ContentBlock[], over: Partial<ChatMessage> = {}): ChatMessage => ({
  role: 'agent', from: 'ling', to: 'user', text: '', timestamp: '', timestampMs: 0,
  runId, live: true, seenSaved: 0, content, ...over,
});

/** A saved row of run `runId`, as the history view brings it. */
const row = (runId: string, text: string, tools: ContentBlock[] = [], over: Partial<ChatMessage> = {}): ChatMessage => ({
  role: 'agent', from: 'ling', to: 'user', text, timestamp: '', timestampMs: 0,
  runId, content: tools, ...over,
});

test("a stopped run's late events never reach the next turn's bubble", () => {
  // Run a stopped with its Bash still showing running; the next turn opened
  // a placeholder before its run began.
  const stopped = bubble('a', [call('Bash', 'running')], { isGenerating: true });
  const placeholder: ChatMessage = {
    role: 'agent', from: 'ling', to: 'user', text: 'Model loading...', timestamp: '', isGenerating: true,
  };
  const msgs = [user('Draw a half-auto one.', 1), stopped, user('Draw a full-auto one.', 2), placeholder];
  // Run b's first event takes the placeholder, not run a's bubble.
  const b = runBubbleIndex(msgs, 'ling', 'b');
  assert.equal(b, 3);
  const claimed = claimForRun(msgs, b, 'b');
  assert.equal(claimed[3].runId, 'b');
  // Run a's own end finds run a's bubble; run b's finds run b's.
  assert.equal(runBubbleIndex(claimed, 'ling', 'a'), 1);
  assert.equal(runBubbleIndex(claimed, 'ling', 'b'), 3);
  // An event no run sent never lands in a run's bubble.
  assert.equal(runBubbleIndex([user('x', 1), stopped], 'ling'), -1);
});

test('each finished run shows as its own bubble, marked as its rows say', () => {
  const a = bubble('a', [call('Bash', 'running')]);
  const b = bubble('b', [call('Bash', 'done'), words('Drawn this time.')], { text: 'Drawn this time.' });
  const shown = bindRuns([
    user('Draw a half-auto one.', 1),
    row('a', '', [call('Bash', 'failed')], { interrupted: true, runEndedMs: 9 }),
    a,
    user('Draw a full-auto one.', 2),
    row('b', 'Drawn this time.', [call('Bash', 'done')]),
    b,
  ]);
  assert.equal(shown.length, 4);
  assert.equal(shown[1].runId, 'a');
  assert.equal(shown[1].interrupted, true);
  assert.equal(shown[1].content?.[0].status, 'failed', 'a stopped call never came back');
  assert.equal(shown[3].runId, 'b');
  assert.equal(shown[3].interrupted, undefined);
  assert.equal(shown[3].text, 'Drawn this time.');
});

test('after a reload every run shows its saved rows', () => {
  const rows = [
    user('List the notes.', 1),
    row('a', 'Looking first.', [call('Glob', 'done')]),
    row('a', 'Two notes here.'),
  ];
  assert.deepEqual(bindRuns(rows), rows);
});

test('a bubble that took the run mid-way shows after the rows it did not see', () => {
  const msgs = [user('List the notes.', 1), row('a', 'Looking first.', [call('Glob', 'done')])];
  const fields = bubbleOfRun(msgs, 'a');
  assert.equal(fields.seenSaved, 1);
  const live = bubble('a', [call('Read', 'running')], { ...fields, isGenerating: true });
  // A row the run saved after the bubble took it is the bubble's.
  const later = row('a', 'Reading one.');
  const shown = bindRuns([...msgs, later, live]);
  assert.deepEqual(shown.map((m) => m.text), ['List the notes.', 'Looking first.', '']);
});

test('a finished bubble that missed part of its run gives way to the rows', () => {
  // Away past the replay window: the bubble never saw the first words.
  const missed = bubble('a', [call('Glob', 'done'), words('Two notes here.')], { text: 'Two notes here.' });
  const rows = [row('a', 'Looking first.', [call('Glob', 'done')]), row('a', 'Two notes here.')];
  const shown = bindRuns([user('List the notes.', 1), ...rows, missed]);
  assert.deepEqual(shown.map((m) => m.text), ['List the notes.', 'Looking first.', 'Two notes here.']);
  assert.ok(shown.every((m) => !m.live));
});

test('a bubble frozen by a reconnect before its reply gives way to the rows', () => {
  const frozen = bubble('a', [call('Glob', 'done')], { wasFrozenStream: true });
  const rows = [row('a', 'Two notes here.', [call('Glob', 'done')])];
  const shown = bindRuns([user('List the notes.', 1), ...rows, frozen]);
  assert.deepEqual(shown.map((m) => m.text), ['List the notes.', 'Two notes here.']);
});

test('rows of another run never bind to a bubble, whatever their words', () => {
  const a = bubble('a', [call('Bash', 'done'), words('Done.')], { text: 'Done.' });
  const other = row('b', 'Done.', [call('Bash', 'done')]);
  const shown = bindRuns([a, other]);
  assert.equal(shown.length, 2);
  assert.deepEqual(shown.map((m) => m.runId), ['a', 'b']);
});

test('a page loaded mid-run shows the live run\'s saved calls, then its bubble after them', () => {
  const row: ChatMessage = { role: 'agent', from: 'ling', text: '', timestamp: '', timestampMs: 2,
    runId: 'r9', content: [call('Read', 'done', 'saved-0'), call('Bash', 'running', 'saved-1')] };
  const msgs = [user('fix it', 1), row];
  const bubble: ChatMessage = { role: 'agent', from: 'ling', text: '', timestamp: '', timestampMs: 3,
    isGenerating: true, content: [call('Edit', 'running')], ...bubbleOfRun(msgs, 'r9') };
  const shown = bindRuns([...msgs, bubble]);
  assert.deepEqual(shown.map((m) => (m.content || []).map((b) => b.tool)), [[], ['Read', 'Bash'], ['Edit']]);
});

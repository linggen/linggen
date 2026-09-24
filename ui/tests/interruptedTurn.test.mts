// node --test ui/tests/*.test.ts (Node strips the types).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { interruptedResend, type TurnRow } from '../src/lib/interruptedTurn.mts';

const UNSPOKEN = new Set(['memory-recall']);
const user = (text: string, ts: number, to = 'ling'): TurnRow => ({ role: 'user', from: 'user', to, text, timestampMs: ts });
const reply = (text: string, ts: number): TurnRow => ({ role: 'agent', from: 'ling', to: 'user', text, timestampMs: ts });

test('an answered turn says nothing', () => {
  assert.equal(interruptedResend([user('hi', 1), reply('hello', 2)], UNSPOKEN, 'ling'), null);
});

test('the last unanswered row is resent to the agent it went to', () => {
  const r = interruptedResend([reply('earlier', 1), user('@银月 你好', 2, 'yinyue')], UNSPOKEN, 'ling');
  assert.deepEqual(r, { text: '@银月 你好', agentId: 'yinyue', persisted: true });
});

test('a recall after the question is not an answer', () => {
  const recall: TurnRow = { role: 'agent', from: 'memory-recall', text: 'From memory…', timestampMs: 3 };
  assert.ok(interruptedResend([user('q', 2), recall], UNSPOKEN, 'ling'));
});

test('tool calls with no words are not a reply', () => {
  const tools: TurnRow = { role: 'agent', from: 'ling', to: 'user', text: '', timestampMs: 3 };
  assert.ok(interruptedResend([user('q', 2), tools], UNSPOKEN, 'ling'));
});

test('a hidden page tap is resent as it was sent', () => {
  const r = interruptedResend([user('[HIDDEN] [scene] won', 5)], UNSPOKEN, 'ling');
  assert.equal(r?.text, '[HIDDEN] [scene] won');
});

test('the same unanswered row earns the line once', () => {
  const q = user('q', 10);
  const line: TurnRow = { role: 'agent', from: 'system', text: 'No response', isError: true, timestampMs: 11, resend: { text: 'q', agentId: 'ling', persisted: true } };
  assert.equal(interruptedResend([q, line], UNSPOKEN, 'ling'), null);
});

test('a resent row that dies again earns a new line', () => {
  const oldLine: TurnRow = { role: 'agent', from: 'system', text: 'No response', isError: true, timestampMs: 11, resend: { text: 'q', agentId: 'ling', persisted: true } };
  assert.ok(interruptedResend([user('q', 10), oldLine, user('q', 20)], UNSPOKEN, 'ling'));
});

test('an empty agent falls back to the selected one', () => {
  assert.equal(interruptedResend([user('q', 1, '')], UNSPOKEN, 'ling')?.agentId, 'ling');
});

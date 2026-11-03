import { test } from 'node:test';
import assert from 'node:assert/strict';
import { ASK_GRACE_MS, enqueueAsk, removeAsk, replaceAsks, settleAsks } from '../src/lib/askQueue.mts';

const ask = (questionId: string, agentId = 'ling') => ({ questionId, agentId, questions: [] });

test('questions queue independently by session and deduplicate by question', () => {
  let queues = enqueueAsk({}, 'a', ask('1'));
  queues = enqueueAsk(queues, 'a', ask('2'));
  queues = enqueueAsk(queues, 'b', ask('1', 'other'));
  queues = enqueueAsk(queues, 'a', ask('1', 'updated'));
  assert.deepEqual(queues.a, [ask('1', 'updated'), ask('2')]);
  assert.deepEqual(queues.b, [ask('1', 'other')]);
});

test('resolving one question does not remove another or affect another session', () => {
  const queues = { a: [ask('1'), ask('2')], b: [ask('1')] };
  assert.deepEqual(removeAsk(queues, 'a', '1'), { a: [ask('2')], b: [ask('1')] });
  assert.deepEqual(removeAsk(queues, 'a', 'missing'), queues);
});

test('restoration accepts every pending question and clears an empty session', () => {
  const queues = replaceAsks({ b: [ask('3')] }, 'a', [ask('1'), ask('2'), ask('1')]);
  assert.deepEqual(queues, { a: [ask('1'), ask('2')], b: [ask('3')] });
  assert.deepEqual(replaceAsks(queues, 'a', []), { a: [], b: [ask('3')] });
});

// Found by tests/e2e (2026-11-03, ~1 in 14 runs): a page_state built before a
// permission ask landed after the ask's live event and took its card away
// while the run waited on it.
test('a page_state that lacks a just-arrived ask keeps it; an old one it lacks goes', () => {
  const now = 1_000_000;
  const live = { ...ask('live'), receivedAtMs: now - 100 };
  const old = { ...ask('old'), receivedAtMs: now - ASK_GRACE_MS };
  const restored = ask('restored');
  assert.deepEqual(settleAsks([live, old, restored], [], now), [live]);
});

test('a page_state keeps the order shown and adds what it lists anew', () => {
  const now = 1_000_000;
  const shown = [{ ...ask('2'), receivedAtMs: now - 100 }, ask('1')];
  assert.deepEqual(settleAsks(shown, [ask('1'), ask('3'), ask('2', 'updated')], now),
    [{ ...ask('2', 'updated'), receivedAtMs: now - 100 }, ask('1'), ask('3')]);
});

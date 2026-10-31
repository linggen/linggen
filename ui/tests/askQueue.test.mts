import { test } from 'node:test';
import assert from 'node:assert/strict';
import { enqueueAsk, removeAsk, replaceAsks } from '../src/lib/askQueue.mts';

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

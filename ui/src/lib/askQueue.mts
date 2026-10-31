import type { PendingAskUser } from '../types';

export type AskQueues = Record<string, PendingAskUser[]>;

export function enqueueAsk(queues: AskQueues, sessionId: string, ask: PendingAskUser): AskQueues {
  const queue = queues[sessionId] ?? [];
  const index = queue.findIndex((item) => item.questionId === ask.questionId);
  const next = [...queue];
  if (index < 0) next.push(ask); else next[index] = ask;
  return { ...queues, [sessionId]: next };
}

export function removeAsk(queues: AskQueues, sessionId: string, questionId: string): AskQueues {
  return { ...queues, [sessionId]: (queues[sessionId] ?? []).filter((ask) => ask.questionId !== questionId) };
}

export function replaceAsks(queues: AskQueues, sessionId: string, asks: PendingAskUser[]): AskQueues {
  let result = { ...queues, [sessionId]: [] };
  for (const ask of asks) result = enqueueAsk(result, sessionId, ask);
  return result;
}

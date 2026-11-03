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

/** An ask this young may be ahead of a page_state it is checked against: the
 *  snapshot can be built before the ask and land after its live event. */
export const ASK_GRACE_MS = 15_000;

/** A session's asks after a page_state: the ones it lists, in the order
 *  already shown, plus asks shown that it lacks but that are too young for it
 *  to know. */
export function settleAsks(shown: PendingAskUser[], listed: PendingAskUser[], now: number): PendingAskUser[] {
  const young = (ask: PendingAskUser) => ask.receivedAtMs !== undefined && now - ask.receivedAtMs < ASK_GRACE_MS;
  const kept = shown.flatMap((ask) => {
    const fresh = listed.find((it) => it.questionId === ask.questionId);
    if (fresh) return [ask.receivedAtMs === undefined ? fresh : { ...fresh, receivedAtMs: ask.receivedAtMs }];
    return young(ask) ? [ask] : [];
  });
  return [...kept, ...listed.filter((it) => !shown.some((ask) => ask.questionId === it.questionId))];
}

export function replaceAsks(queues: AskQueues, sessionId: string, asks: PendingAskUser[]): AskQueues {
  let result = { ...queues, [sessionId]: [] };
  for (const ask of asks) result = enqueueAsk(result, sessionId, ask);
  return result;
}

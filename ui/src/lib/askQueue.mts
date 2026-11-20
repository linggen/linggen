import type { PendingAskUser } from '../types';
import { toldOf } from './toldAloud.mts';

export type AskQueues = Record<string, PendingAskUser[]>;

/** An open question as the engine lists it (the page state, the pending
 *  list) — its place only when the engine told one. */
export type ListedAsk = { question_id: string; agent_id?: string; questions?: unknown[]; place?: string | null; told_aloud_at?: number; told_aloud_line?: string };

/** Whether a waiting question is a permission prompt (its card is the
 *  permission card) rather than a question of the agent's. */
export function isPermissionAsk(ask: PendingAskUser): boolean {
  const q = ask.questions[0];
  return !!q?.permission || q?.header === 'Permission';
}

/** A listed question as the page keeps it. */
export function listedAsk(it: ListedAsk): PendingAskUser {
  const ask: PendingAskUser = {
    questionId: it.question_id,
    agentId: String(it.agent_id || ''),
    questions: (it.questions || []) as PendingAskUser['questions'],
  };
  if (it.place) ask.place = it.place;
  const told = toldOf(it);
  if (told) ask.toldAloud = told;
  return ask;
}

export function enqueueAsk(queues: AskQueues, sessionId: string, ask: PendingAskUser): AskQueues {
  const queue = queues[sessionId] ?? [];
  const index = queue.findIndex((item) => item.questionId === ask.questionId);
  const next = [...queue];
  if (index < 0) next.push(ask); else next[index] = ask;
  return { ...queues, [sessionId]: next };
}

/** The waiting question `questionId` in `sessionId`, marked: her notice of
 *  its wait was said aloud (`lib/toldAloud.mts`). */
export function markAsk(queues: AskQueues, sessionId: string, questionId: string, told: PendingAskUser['toldAloud']): AskQueues {
  const queue = queues[sessionId];
  if (!queue?.some((ask) => ask.questionId === questionId)) return queues;
  return { ...queues, [sessionId]: queue.map((ask) => (ask.questionId === questionId ? { ...ask, toldAloud: told } : ask)) };
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

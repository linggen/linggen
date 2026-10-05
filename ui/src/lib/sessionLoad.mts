/**
 * A load or reply that was asked for one session belongs to that session.
 *
 * Session loads and sends are async: the person can switch chats (or start a
 * new one) before the answer comes back. Writing a late answer into whatever
 * chat is open then shows one session's rows in another — the old session's
 * thread flashed into a fresh New chat (found by tests/e2e, 2026-10-05). Each
 * async path keeps the session it asked for and checks it here before it
 * writes; a stale answer is dropped (the session's own load runs again when
 * it is opened).
 */

/** The session a load or send was asked for is still the one on screen. */
export function stillOn(asked: string | null | undefined, open: string | null | undefined): boolean {
  return (asked || null) === (open || null);
}

/** What a session-state load saw — enough to tell whether it changed. */
export type LoadSnapshot = {
  sessionId: string;
  messageCount: number;
  agentStatus?: unknown;
  planStatus?: unknown;
};

/**
 * What to do with a session-state load that just came back:
 *  - `stale`: the person moved to another session — drop it;
 *  - `unchanged`: the same session as last applied, nothing new — skip
 *    (avoids re-render loops);
 *  - `apply`: write it.
 * `previous` is the last load applied; a load of another session is never
 * `unchanged`, however alike their counts.
 */
export function loadOutcome(
  loaded: LoadSnapshot,
  open: string | null | undefined,
  previous: LoadSnapshot | null,
): 'stale' | 'unchanged' | 'apply' {
  if (!stillOn(loaded.sessionId, open)) return 'stale';
  if (
    previous
    && previous.sessionId === loaded.sessionId
    && previous.messageCount === loaded.messageCount
    && previous.agentStatus === loaded.agentStatus
    && previous.planStatus === loaded.planStatus
  ) return 'unchanged';
  return 'apply';
}

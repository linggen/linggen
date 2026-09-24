/**
 * The "no response" line after a turn died without a reply — pure, so the
 * judgment is testable without a store.
 */

/** The fields of a chat row this judgment reads. */
export interface TurnRow {
  role: string;
  from?: string;
  to?: string;
  text: string;
  timestampMs?: number;
  isError?: boolean;
  images?: string[];
  resend?: { text: string; agentId: string; images?: string[]; persisted?: boolean };
}

export interface InterruptedResend {
  text: string;
  agentId: string;
  images?: string[];
  persisted: true;
}

/**
 * The resend payload for the last user row nothing answered, or null when
 * there is nothing to say. `unspoken` are senders whose rows are not a reply
 * (a memory recall lands right after the user's message).
 *
 * A turn is judged from more than one place (a reconnect, a stale optimistic
 * flag, a later page_state), so the same unanswered row must earn the line
 * once: a line already standing for it — or for a resend of it — means null.
 */
export function interruptedResend(
  rows: readonly TurnRow[],
  unspoken: ReadonlySet<string>,
  fallbackAgent: string,
): InterruptedResend | null {
  // A turn killed after its tool calls leaves rows with no words: those are
  // work, not a reply.
  const spoken = rows.filter(
    (m) => !m.isError && !unspoken.has(m.from || '') && (m.role === 'user' || m.text.trim() !== ''),
  );
  const last = spoken[spoken.length - 1];
  if (!last || last.role !== 'user') return null;
  const since = last.timestampMs ?? 0;
  const alreadySaid = rows.some(
    (m) => m.isError && m.resend?.text === last.text && (m.timestampMs ?? 0) >= since,
  );
  if (alreadySaid) return null;
  return {
    text: last.text,
    agentId: last.to || fallbackAgent,
    ...(last.images && last.images.length > 0 ? { images: last.images } : {}),
    persisted: true,
  };
}

/** Said for a turn that ended with no reply: the daemon restarted, or a run
 *  that hung was reaped. Either way the run stopped — not always a restart. */
export const INTERRUPTED_TEXT = 'No response — the run stopped before it replied.';

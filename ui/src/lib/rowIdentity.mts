/**
 * When two chat rows are the same row. A person may say the same words twice
 * ("ok", "continue", "restart game"); each saying is its own row and shows.
 * Only an echo — one row delivered twice — collapses:
 *  - rows that both carry a `clientId` are the same row only by that id;
 *  - otherwise the same sender, recipient and words, AND timing that says the
 *    second copy is this row again, never a later repeat.
 * Pure, so node tests can hold it.
 */
import type { ChatMessage } from '../types.ts';

/** A saved row carries seconds; a live event carries ms. */
const SAME_MOMENT_MS = 2_000;
/** A row's saved copy is never older than its live copy (seconds rounding aside). */
const SAVED_COPY_SLACK_MS = 5_000;

export const normalizeMessageTextForDedup = (text?: string) =>
  (text || '').replace(/\s+/g, ' ').trim();

/** Strip run-ID prefix (e.g. "run-ling-1234" → "ling") so streaming messages
 *  (which carry the run ID as `from`) can match persisted messages (which use
 *  the bare agent name). */
export const normalizeFrom = (from: string): string => {
  const m = from.match(/^run-(.+?)-\d+$/);
  return m ? m[1] : from;
};

const senderOf = (m: ChatMessage) => normalizeFrom(m.from || m.role);
const tsOf = (m: ChatMessage) => m.timestampMs ?? 0;

/** Same sender, recipient and words. */
export const sameMessageContent = (a: ChatMessage, b: ChatMessage): boolean =>
  senderOf(a) === senderOf(b) &&
  (a.to || '') === (b.to || '') &&
  normalizeMessageTextForDedup(a.text) === normalizeMessageTextForDedup(b.text);

/** Both rows carry a sent-message id: the id alone decides. */
export const idsDecide = (a: ChatMessage, b: ChatMessage): boolean =>
  !!a.clientId && !!b.clientId;

const sameMoment = (a: ChatMessage, b: ChatMessage): boolean =>
  !tsOf(a) || !tsOf(b) || Math.abs(tsOf(a) - tsOf(b)) <= SAME_MOMENT_MS;

/**
 * A live row this client holds that is a saved/kept row again. Its saved copy
 * is stamped when the row finished — at or after the live copy began, even
 * minutes after for a long turn — so an earlier row with the same words is a
 * repeat, not this row.
 */
export const isLiveEcho = (live: ChatMessage, kept: ChatMessage): boolean => {
  if (idsDecide(live, kept)) return live.clientId === kept.clientId;
  if (!sameMessageContent(live, kept)) return false;
  if (!tsOf(live) || !tsOf(kept)) return true;
  return tsOf(kept) >= tsOf(live) - SAVED_COPY_SLACK_MS;
};

/** The agent's latest finished row, if nothing was asked of that agent since. */
const lastAnswerOf = (rows: ChatMessage[], sender: string): ChatMessage | null => {
  for (let i = rows.length - 1; i >= 0; i -= 1) {
    const row = rows[i];
    if (normalizeFrom(row.to || '') === sender) return null; // a new turn began
    if (senderOf(row) === sender && !row.isGenerating) return row;
  }
  return null;
};

/**
 * A `message` event that is a row already on screen. By id when it carries
 * one; else the same row at the same moment (a replayed event), or — for an
 * agent — its last finished answer, which `turn_complete` may have finalized
 * before the event came (stamped when it began, so the times differ).
 */
export const isEventEcho = (rows: ChatMessage[], incoming: ChatMessage): boolean => {
  if (incoming.clientId) return rows.some((r) => r.clientId === incoming.clientId);
  const sameAtOnce = rows.some((r) =>
    !r.isGenerating && !r.clientId && sameMessageContent(r, incoming) && sameMoment(r, incoming));
  if (sameAtOnce) return true;
  const sender = senderOf(incoming);
  if (sender === 'user') return false;
  const last = lastAnswerOf(rows, sender);
  return !!last && sameMessageContent(last, incoming);
};

/**
 * The final pass over a merge: every saved row shows; a live row shows unless
 * it echoes a row already kept. A live row still generating always shows.
 */
export const dropLiveEchoes = (saved: ChatMessage[], live: ChatMessage[]): ChatMessage[] => {
  const kept = [...saved];
  for (const msg of live) {
    const echo = !msg.isGenerating && !msg.interrupted && kept.some((k) => isLiveEcho(msg, k));
    if (!echo) kept.push(msg);
  }
  return kept;
};

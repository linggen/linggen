/**
 * A person's message and its bubble. The surface gives the bubble an id
 * (`clientId`) and sends it with the message (`client_id`); the engine keeps
 * it on the saved row and echoes it on that row's `message` event. The
 * bubble is matched to the row by that id — never by its words, which differ
 * once the engine keeps only the words after a leading `@name`. Pure, so
 * node tests can hold it.
 */
import type { ChatMessage } from '../types.ts';

/** A fresh id for a bubble this surface is about to send. */
export const newClientId = (): string =>
  `c-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;

/** Two rows of the same sent message: both carry the same bubble id. */
export const sameSentMessage = (a: ChatMessage, b: ChatMessage): boolean =>
  !!a.clientId && a.clientId === b.clientId;

/** The bubble with `clientId`, now as the engine kept it — the saved words
 *  and whom it went to. `null` when no bubble here has that id (another
 *  surface sent it): the event is then a new row like any other. */
export const confirmSent = (
  msgs: ChatMessage[],
  clientId: string,
  text: string,
  to: string,
): ChatMessage[] | null => {
  const idx = msgs.findIndex((m) => m.clientId === clientId);
  if (idx < 0) return null;
  const next = [...msgs];
  next[idx] = { ...msgs[idx], text, ...(to ? { to } : {}) };
  return next;
};

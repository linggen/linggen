/**
 * The page's list of queued messages, one list across the session's agents.
 * Pure, so node tests can hold it; the stores and ChatInput apply it.
 *
 * The engine owns the queue. The page only hides a row while a remove or
 * steer is on its way, and puts it back when the engine never heard it.
 */

/** The fields this list needs of a queued message. */
export type Queued = { id: string; agent_id: string };

/** The list after one agent's queue changed: that agent's rows replaced by
 *  `items`, every other agent's rows kept as they were. */
export function withAgentQueue<T extends Queued>(list: T[], agentId: string, items: T[]): T[] {
  return [...list.filter((q) => q.agent_id !== agentId), ...items];
}

/** The list without the message `id`, and where it stood (-1 when absent). */
export function withoutQueued<T extends Queued>(list: T[], id: string): { list: T[]; at: number } {
  return { list: list.filter((q) => q.id !== id), at: list.findIndex((q) => q.id === id) };
}

/** The list with `item` back where it stood, unless it is there already. */
export function restoreQueued<T extends Queued>(list: T[], item: T, at: number): T[] {
  if (list.some((q) => q.id === item.id)) return list;
  const place = Math.max(0, Math.min(at, list.length));
  return [...list.slice(0, place), item, ...list.slice(place)];
}

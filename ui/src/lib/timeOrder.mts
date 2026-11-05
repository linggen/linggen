/**
 * The chat's one time order. Saved rows carry their place in the session's
 * history (`rowOrder`): rows saved in the same second — a message a running
 * turn took in and the reply around it — keep the order the engine stored
 * them in, whatever order a merge produced them in. Anything else that ties
 * keeps its place in the list. Rows with no time go last.
 */
import type { ChatMessage } from '../types.ts';

export function sortByTime(messages: readonly ChatMessage[]): ChatMessage[] {
  return messages
    .map((msg, index) => ({ msg, index }))
    .sort((a, b) => {
      const ta = a.msg.timestampMs ?? 0;
      const tb = b.msg.timestampMs ?? 0;
      if (ta <= 0 && tb <= 0) return a.index - b.index;
      if (ta <= 0) return 1;
      if (tb <= 0) return -1;
      if (ta !== tb) return ta - tb;
      const ra = a.msg.rowOrder;
      const rb = b.msg.rowOrder;
      if (ra != null && rb != null && ra !== rb) return ra - rb;
      return a.index - b.index;
    })
    .map((entry) => entry.msg);
}

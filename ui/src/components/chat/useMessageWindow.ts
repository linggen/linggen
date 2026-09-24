// How much of a long conversation the list renders: the last PAGE messages,
// and PAGE more each time "Load earlier" is pressed. The window's start is
// fixed once set, so rows arriving at the bottom never push rows out of the
// top; it re-anchors when the session changes, when its history shrinks
// below it (cleared, compacted) or when a whole page or more arrives since it
// was set (the history loaded in bulk — streaming adds a row or two at a time).
import { useCallback, useLayoutEffect, useRef, useState } from 'react';

export const MESSAGE_PAGE = 100;

export function useMessageWindow(
  count: number,
  sessionId: string | null | undefined,
  scrollRef: React.RefObject<HTMLDivElement | null>,
) {
  const [win, setWin] = useState(() => ({ sid: sessionId, from: Math.max(0, count - MESSAGE_PAGE), seen: count }));
  let from = win.from;
  if (win.sid !== sessionId || count < win.from || count - win.seen > MESSAGE_PAGE) {
    from = Math.max(0, count - MESSAGE_PAGE);
    setWin({ sid: sessionId, from, seen: count });
  }

  // Loading earlier rows grows the list above the viewport; keep what the
  // reader was looking at in place by holding the distance from the bottom.
  const keepFromBottom = useRef<number | null>(null);
  const loadEarlier = useCallback(() => {
    const el = scrollRef.current;
    keepFromBottom.current = el ? el.scrollHeight - el.scrollTop : null;
    setWin((w) => ({ ...w, from: Math.max(0, w.from - MESSAGE_PAGE) }));
  }, [scrollRef]);
  useLayoutEffect(() => {
    const el = scrollRef.current;
    const kept = keepFromBottom.current;
    if (!el || kept == null) return;
    keepFromBottom.current = null;
    el.scrollTop = el.scrollHeight - kept;
  }, [from, scrollRef]);

  return { from, loadEarlier };
}

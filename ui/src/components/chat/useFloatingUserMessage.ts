// The sticky "You: …" banner: the nearest user message scrolled above the
// top of the chat.
import { useEffect, useRef, useState } from 'react';
import type { ChatMessage } from '../../types';

const BANNER_HEIGHT = 48;

export function useFloatingUserMessage(
  scrollRef: React.RefObject<HTMLDivElement | null>,
  userMsgRefs: React.RefObject<Map<number, HTMLDivElement>>,
  messages: ChatMessage[],
  sessionId: string | null | undefined,
) {
  const [floating, setFloating] = useState<{ index: number; text: string } | null>(null);

  // A new session starts without the previous one's banner.
  useEffect(() => {
    setFloating(null);
    userMsgRefs.current?.clear();
  }, [sessionId, userMsgRefs]);

  const messagesRef = useRef(messages);
  useEffect(() => { messagesRef.current = messages; }, [messages]);

  useEffect(() => {
    const container = scrollRef.current;
    if (!container) return;
    let rafId = 0;
    const update = () => {
      const threshold = container.getBoundingClientRect().top + BANNER_HEIGHT;
      let bestIdx = -1;
      let bestTop = -Infinity;
      for (const [idx, el] of userMsgRefs.current?.entries() ?? []) {
        const top = el.getBoundingClientRect().top;
        if (top < threshold && top > bestTop) {
          bestTop = top;
          bestIdx = idx;
        }
      }
      const msg = bestIdx >= 0 ? messagesRef.current[bestIdx] : undefined;
      setFloating((prev) => {
        if (!msg) return prev === null ? prev : null;
        if (prev?.index === bestIdx && prev.text === msg.text) return prev;
        return { index: bestIdx, text: msg.text };
      });
    };
    const onScroll = () => {
      cancelAnimationFrame(rafId);
      rafId = requestAnimationFrame(update);
    };
    container.addEventListener('scroll', onScroll, { passive: true });
    update();
    return () => {
      container.removeEventListener('scroll', onScroll);
      cancelAnimationFrame(rafId);
    };
  }, [scrollRef, userMsgRefs]);

  return floating;
}

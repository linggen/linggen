/**
 * EventSource lifecycle: connection, seq dedup, reconnect.
 */
import { useEffect, useRef } from 'react';
import type { UiSseMessage } from '../types';

export interface SseConnectionOptions {
  onEvent: (item: UiSseMessage) => void;
  onParseError?: () => void;
  /** When provided, the SSE connection passes session_id to the server for
   *  server-side filtering. The connection reconnects when this value changes. */
  sessionId?: string | null;
}

export function useSseConnection({ onEvent, onParseError, sessionId }: SseConnectionOptions) {
  const lastSeqRef = useRef(0);
  // Use refs so the EventSource doesn't need to be recreated when callbacks change
  const onEventRef = useRef(onEvent);
  const onParseErrorRef = useRef(onParseError);

  useEffect(() => {
    onEventRef.current = onEvent;
    onParseErrorRef.current = onParseError;
  }, [onEvent, onParseError]);

  useEffect(() => {
    const url = sessionId
      ? `/api/events?session_id=${encodeURIComponent(sessionId)}`
      : '/api/events';
    const events = new EventSource(url);
    // Reset seq counter on (re)connect. This is safe because:
    // 1. Server restart resets seq to 0, so old seq values won't collide.
    // 2. EventSource auto-reconnects on disconnect, and we need to accept
    //    the server's new seq range without stale filtering.
    events.onopen = () => {
      lastSeqRef.current = 0;
    };
    events.onmessage = (e) => {
      try {
        const item = JSON.parse(e.data) as UiSseMessage;
        if (typeof item.seq === 'number') {
          if (item.seq <= lastSeqRef.current) return;
          lastSeqRef.current = item.seq;
        }
        onEventRef.current(item);
      } catch (err) {
        console.error('SSE parse error', err);
        onParseErrorRef.current?.();
      }
    };
    return () => events.close();
  }, [sessionId]);
}

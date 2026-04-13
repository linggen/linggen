/**
 * React hook for the Transport abstraction.
 *
 * Manages transport lifecycle (connect, subscribe, disconnect) and
 * routes inbound events through the event dispatcher pipeline.
 *
 * Transport: WebRTC (all Web UI — local and remote).
 */
import { useEffect, useRef } from 'react';
import { getTransport, setTransport, type Transport, type TransportCallbacks, type TransportStatus } from '../lib/transport';
import { RtcTransport } from '../lib/rtcTransport';
import { RelaySignaling } from '../lib/signaling';
import { dispatchEvent } from '../lib/eventDispatcher';
import { useUserStore } from '../stores/userStore';
import { useChatStore } from '../stores/chatStore';
import { useSessionStore } from '../stores/sessionStore';

/** Send the frontend's active view context to the server.
 *  The server uses this to scope its page_state push. */
export function sendViewContext() {
  try {
    const transport = getTransport();
    const { activeSessionId, selectedProjectRoot } = useSessionStore.getState();
    const isCompact = new URLSearchParams(window.location.search).get('mode') === 'compact';
    transport.sendViewContext({
      sessionId: activeSessionId,
      projectRoot: selectedProjectRoot,
      isCompact,
    });
  } catch { /* transport not ready */ }
}


/** Map transport status to the UI store's connection status values. */
function mapStatus(status: TransportStatus): 'connected' | 'reconnecting' | 'disconnected' {
  switch (status) {
    case 'connected': return 'connected';
    case 'reconnecting': return 'reconnecting';
    default: return 'disconnected';
  }
}

/** Get remote instance ID from URL or injected meta tag (tunnel mode). */
function getInstanceId(): string | null {
  const params = new URLSearchParams(window.location.search);
  return params.get('instance')
    || window.location.pathname.match(/\/connect\/([^/]+)/)?.[1]
    || document.querySelector('meta[name="linggen-instance"]')?.getAttribute('content')
    || null;
}

export interface UseTransportOptions {
  sessionId?: string | null;
  onReconnect?: () => void;
  onParseError?: () => void;
}

/**
 * Connects the global transport and subscribes to a session.
 * Events are dispatched through the event dispatcher — all existing event handling stays the same.
 */
export function useTransport({ sessionId, onReconnect, onParseError }: UseTransportOptions) {
  const onReconnectRef = useRef(onReconnect);
  const onParseErrorRef = useRef(onParseError);
  const sessionIdRef = useRef(sessionId);

  useEffect(() => {
    onReconnectRef.current = onReconnect;
    onParseErrorRef.current = onParseError;
  }, [onReconnect, onParseError]);

  useEffect(() => {
    sessionIdRef.current = sessionId;
  }, [sessionId]);

  // Initialize transport once
  useEffect(() => {
    const callbacks: TransportCallbacks = {
      onEvent: (_sid, event) => {
        dispatchEvent(event, sessionIdRef.current ?? undefined);
      },
      onStatusChange: (status) => {
        useUserStore.getState().setConnectionStatus(mapStatus(status));
      },
      onReconnect: () => {
        // Send view context to trigger server-pushed page_state
        sendViewContext();
        // Fetch workspace state immediately (chat history — not included in page_state)
        // Skip for consumer mode — HTTP fetch blocked by WebRTC tunnel permissions.
        useChatStore.getState().fetchSessionState();
        if (onReconnectRef.current) {
          onReconnectRef.current();
        }
      },
      onParseError: () => {
        onParseErrorRef.current?.();
      },
    };

    // Check if transport already exists (another useTransport call may have created it)
    let transport: Transport;
    let created = false;
    try {
      transport = getTransport();
      // Transport exists — reconnect if it was disconnected (React strict mode
      // runs effects twice: mount→cleanup→mount, so the first cleanup disconnects
      // and the second mount must reconnect).
      if (transport.status() === 'disconnected') {
        transport.connect();
      }
    } catch {
      // No transport yet — create one (always WebRTC)
      const instanceId = getInstanceId();
      const config = instanceId
        ? { signaling: new RelaySignaling(instanceId) }
        : {};
      transport = new RtcTransport(callbacks, config);
      setTransport(transport);
      created = true;
      transport.connect();
    }

    if (!created) return;

    return () => {
      transport.disconnect();
    };
  }, []);

  // Subscribe to session changes (and unsubscribe old session)
  useEffect(() => {
    if (!sessionId) return;
    try {
      const transport = getTransport();
      transport.subscribeSession(sessionId);
      return () => {
        transport.unsubscribeSession(sessionId);
      };
    } catch {
      // Transport not initialized yet — will subscribe on next render
    }
  }, [sessionId]);
}

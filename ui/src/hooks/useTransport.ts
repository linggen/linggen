/**
 * React hook for the Transport abstraction.
 *
 * Manages transport lifecycle (connect, subscribe, disconnect) and
 * routes inbound events through the event dispatcher pipeline.
 *
 * Transport selection:
 * - Default: SSE (existing behavior)
 * - When URL has ?transport=webrtc or the page is served from a remote host: WebRTC
 */
import { useEffect, useRef } from 'react';
import { SseTransport, getTransport, setTransport, type TransportCallbacks, type TransportStatus } from '../lib/transport';
import { RtcTransport } from '../lib/rtcTransport';
import { dispatchEvent } from '../lib/eventDispatcher';
import { useUiStore } from '../stores/uiStore';
import { useChatStore } from '../stores/chatStore';
import { useAgentStore } from '../stores/agentStore';
import { useProjectStore } from '../stores/projectStore';

/** Refetch all critical state after a reconnect to fill any gaps. */
function resyncState() {
  useChatStore.getState().fetchWorkspaceState();
  useAgentStore.getState().fetchAgentRuns();
  useProjectStore.getState().fetchSessions();
  useProjectStore.getState().fetchAllAgentTrees();
  useUiStore.getState().fetchPendingAskUser();
}

/** Map transport status to the UI store's SSE status values. */
function mapStatus(status: TransportStatus): 'connected' | 'reconnecting' | 'disconnected' {
  switch (status) {
    case 'connected': return 'connected';
    case 'reconnecting': return 'reconnecting';
    default: return 'disconnected';
  }
}

/** Detect whether to use WebRTC transport. */
function shouldUseWebRTC(): boolean {
  const params = new URLSearchParams(window.location.search);
  // Explicit opt-in via URL parameter
  if (params.get('transport') === 'webrtc') return true;
  // Explicit opt-out
  if (params.get('transport') === 'sse') return false;
  // Remote host (not localhost) — use WebRTC
  const host = window.location.hostname;
  if (host !== 'localhost' && host !== '127.0.0.1' && !host.startsWith('192.168.') && !host.startsWith('10.')) {
    return true;
  }
  return false;
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
        useUiStore.getState().setConnectionStatus(mapStatus(status));
      },
      onReconnect: () => {
        if (onReconnectRef.current) {
          onReconnectRef.current();
        } else {
          resyncState();
        }
      },
      onParseError: () => {
        onParseErrorRef.current?.();
      },
    };

    let transport;
    if (shouldUseWebRTC()) {
      transport = new RtcTransport(callbacks);
      setTransport(transport);
    } else {
      transport = getTransport(callbacks);
    }

    transport.connect();

    return () => {
      transport.disconnect();
    };
  }, []);

  // Subscribe to session changes
  useEffect(() => {
    try {
      const transport = getTransport();
      if (sessionId) {
        transport.subscribeSession(sessionId);
      }
    } catch {
      // Transport not initialized yet — will subscribe on next render
    }
  }, [sessionId]);
}

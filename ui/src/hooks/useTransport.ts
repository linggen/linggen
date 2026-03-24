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
import { getTransport, setTransport, type TransportCallbacks, type TransportStatus } from '../lib/transport';
import { RtcTransport } from '../lib/rtcTransport';
import { RelaySignaling } from '../lib/signaling';
import { dispatchEvent } from '../lib/eventDispatcher';
import { useUiStore } from '../stores/uiStore';
import { useChatStore } from '../stores/chatStore';
import { useAgentStore } from '../stores/agentStore';
import { useProjectStore } from '../stores/projectStore';

/** Refetch all critical state after connect/reconnect to fill any gaps. */
function resyncState() {
  useProjectStore.getState().fetchProjects();
  useProjectStore.getState().fetchSessions();
  useProjectStore.getState().fetchAllSessions();
  useProjectStore.getState().fetchAllAgentTrees();
  useAgentStore.getState().fetchModels();
  useAgentStore.getState().fetchDefaultModels();
  useAgentStore.getState().fetchSkills();
  useAgentStore.getState().fetchAgentRuns();
  useChatStore.getState().fetchWorkspaceState();
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
  // Remote mode (instance param present) — always WebRTC
  if (getInstanceId()) return true;
  // Remote host (not localhost) — use WebRTC
  const host = window.location.hostname;
  if (host !== 'localhost' && host !== '127.0.0.1' && !host.startsWith('192.168.') && !host.startsWith('10.')) {
    return true;
  }
  return false;
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

    // Check if transport already exists (another useTransport call may have created it)
    let transport;
    try {
      transport = getTransport();
      // Transport exists — already initialized by another component
      return;
    } catch {
      // No transport yet — create one
    }

    if (shouldUseWebRTC()) {
      const instanceId = getInstanceId();
      const config = instanceId
        ? { signaling: new RelaySignaling(instanceId) }
        : {};
      transport = new RtcTransport(callbacks, config);
      setTransport(transport);
    } else {
      transport = getTransport(callbacks);
    }

    transport.connect();

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

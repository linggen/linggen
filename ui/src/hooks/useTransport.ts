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
import { useUiStore } from '../stores/uiStore';
import { RtcTransport } from '../lib/rtcTransport';
import { RelaySignaling } from '../lib/signaling';
import { dispatchEvent } from '../lib/eventDispatcher';
import { useUserStore } from '../stores/userStore';
import { useChatStore } from '../stores/chatStore';
import { useSessionStore } from '../stores/sessionStore';
import { useServerStore } from '../stores/serverStore';

/** Send the frontend's active view context to the server.
 *  The server uses this to scope its page_state push. */
export function sendViewContext() {
  try {
    const transport = getTransport();
    const { activeSessionId, selectedProjectRoot } = useSessionStore.getState();
    // View is baked in by the entry file (main.tsx / embed.tsx / consumer.tsx)
    // via window.__LINGGEN_VIEW__. Defaults to 'main' for safety.
    const view = ((window as any).__LINGGEN_VIEW__ || 'main') as 'main' | 'embed' | 'consumer';
    transport.sendViewContext({
      sessionId: activeSessionId,
      projectRoot: selectedProjectRoot,
      view,
    });
  } catch { /* transport not ready */ }
}


/** Sticky module-scoped intent: does this page want to present Yinyue? Set by
 *  `useYinyuePresenter`. Read in `onReconnect` (below) so the subscribe is sent
 *  AFTER the transport connects — a child's mount effect runs before Root's
 *  transport-creating effect, so a subscribe at mount time would be lost. */
let yinyuePresenterWanted = false;

/** Join / leave the server's Yinyue presenter registry (FCFS singleton lock). */
function sendYinyueSubscribe() {
  try { getTransport().sendYinyueSubscribe?.(); } catch { /* transport not ready — onReconnect re-sends */ }
}
function sendYinyueRelease() {
  try { getTransport().sendYinyueRelease?.(); } catch { /* transport not ready */ }
}

/**
 * Subscribe this surface to the Yinyue presenter lock and report whether it
 * currently holds it. Only the holder should render her avatar + play her voice
 * — so with multiple tabs/apps open she appears in exactly one place (FCFS;
 * released on close, the next surface takes over). Pass `enabled = false` for
 * surfaces that must never present her (e.g. a branded app's main window, where
 * the native pet window owns her).
 */
export function useYinyuePresenter(enabled = true): boolean {
  const present = useUiStore((s) => s.yinyuePresenter);
  useEffect(() => {
    yinyuePresenterWanted = enabled;
    if (!enabled) return;
    // Best-effort now (covers route changes where the transport already exists);
    // the real guarantee is the re-send in onReconnect once connected.
    sendYinyueSubscribe();
    const release = () => sendYinyueRelease();
    // Explicit release on tab/window close → instant handoff (the server's
    // disconnect timeout is just the crash fallback).
    window.addEventListener('beforeunload', release);
    return () => {
      window.removeEventListener('beforeunload', release);
      yinyuePresenterWanted = false;
      sendYinyueRelease();
    };
  }, [enabled]);
  return enabled && present;
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
        // Re-assert Yinyue presenter candidacy now that we're connected (the
        // mount-time subscribe is lost if it raced ahead of transport creation).
        if (yinyuePresenterWanted) {
          try { getTransport().sendYinyueSubscribe?.(); } catch { /* not ready */ }
        }
        // Fetch workspace state immediately (chat history — not included in page_state)
        // Skip for consumer mode — HTTP fetch blocked by WebRTC tunnel permissions.
        useChatStore.getState().fetchSessionState();
        if (onReconnectRef.current) {
          onReconnectRef.current();
        }
      },
      onParseError: () => {
        // Generic recovery: transport failed to parse a message — refetch the
        // chat state and agent runs to re-sync. Any app-specific handler
        // registered via useTransport runs after.
        useChatStore.getState().fetchSessionState();
        useServerStore.getState().fetchAgentRuns();
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

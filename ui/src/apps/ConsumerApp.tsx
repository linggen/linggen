/**
 * Consumer entry — surface for a remote consumer joining a proxy room.
 * Delegates to <ConsumerChatPage> for the full chat experience with
 * room-scoped skills, shared models, and privacy warnings.
 */
import React, { useEffect, useRef } from 'react';
import { ConsumerChatPage } from '../components/ConsumerChatPage';
import { ToastContainer } from '../components/ToastContainer';
import { useSessionStore } from '../stores/sessionStore';
import { useUserStore } from '../stores/userStore';
import { useChatStore } from '../stores/chatStore';
import { useInteractionStore } from '../stores/interactionStore';
import { useServerStore } from '../stores/serverStore';
import { sendViewContext } from '../hooks/useTransport';

/** Detect remote/tunnel mode (blob iframe with injected instance meta tag). */
const isRemoteMode = typeof document !== 'undefined' && !!document.querySelector('meta[name="linggen-instance"]');

export const ConsumerApp: React.FC = () => {
  const activeSessionId = useSessionStore((s) => s.activeSessionId);

  // Re-subscribe the chat widget when the user picks a session in the sidebar.
  // Without this, setActiveSessionId updates the store but the chat widget
  // never re-fetches message history or clears transient UI state.
  const prevSidRef = useRef<string | null>(null);
  useEffect(() => {
    if (!activeSessionId) return;
    const prev = prevSidRef.current;
    prevSidRef.current = activeSessionId;

    const cs = useChatStore.getState();
    cs.setActiveSession(activeSessionId);

    if (prev !== null && prev !== activeSessionId) {
      const interaction = useInteractionStore.getState();
      interaction.setQueuedMessages([]);
      interaction.setActivePlan(null);
      interaction.setPendingPlan(null);
      interaction.setPendingPlanAgentId(null);
      useServerStore.getState().setAgentStatusText((p) => { const n = { ...p }; delete n[prev]; return n; });
    }
    cs.fetchSessionState();
    sendViewContext();
  }, [activeSessionId]);

  const connectionStatus = useUserStore((s) => s.connectionStatus);
  if (isRemoteMode && connectionStatus === 'disconnected') {
    return (
      <div className="h-screen flex items-center justify-center bg-white dark:bg-[#0b0e14]">
        <div className="text-center space-y-4">
          <div className="inline-block animate-spin rounded-full h-10 w-10 border-4 border-slate-200 dark:border-slate-700 border-t-emerald-500" />
          <p className="text-slate-500 dark:text-slate-400 text-sm">Connecting to linggen server…</p>
        </div>
      </div>
    );
  }

  return (
    <>
      <ToastContainer />
      <ConsumerChatPage />
    </>
  );
};

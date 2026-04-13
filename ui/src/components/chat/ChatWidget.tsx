/**
 * Self-contained chat widget — owns transport, actions, run info, auto-scroll.
 * Renders <ChatPanel> with all props derived from stores + hooks.
 */
import React, { useCallback, useMemo } from 'react';
import { ChatPanel } from './ChatPanel';
import { useTransport } from '../../hooks/useTransport';
import { useChatActions } from '../../hooks/useChatActions';
import { useRunInfo } from '../../hooks/useRunInfo';
import { useAutoScroll } from '../../hooks/useAutoScroll';
import { useSessionStore } from '../../stores/sessionStore';
import { useServerStore } from '../../stores/serverStore';
import { useChatStore } from '../../stores/chatStore';
import { useUiStore } from '../../stores/uiStore';
import { useInteractionStore } from '../../stores/interactionStore';
import { buildSubagentInfos } from '../../lib/messageUtils';

export interface ChatWidgetProps {
  /** Session to connect to. Null/undefined = no session (events from all sessions are accepted). */
  sessionId?: string | null;
  projectRoot?: string;
  mode?: 'full' | 'compact' | 'mobile';
}

export const ChatWidget: React.FC<ChatWidgetProps> = ({
  sessionId,
  projectRoot,
  mode,
}) => {
  // --- Stores ---
  const displayMessages = useChatStore((s) => s.displayMessages);
  const chatMessages = useChatStore((s) => s.messages);
  const agents = useServerStore((s) => s.agents);
  const models = useServerStore((s) => s.models);
  const skills = useServerStore((s) => s.skills);
  const selectedAgent = useServerStore((s) => s.selectedAgent);
  const agentStatus = useServerStore((s) => s.agentStatus);
  const agentContext = useServerStore((s) => s.agentContext);
  const defaultModels = useServerStore((s) => s.defaultModels);
  const cancellingRunIds = useServerStore((s) => s.cancellingRunIds);
  const tokensPerSec = useServerStore((s) => s.tokensPerSec);
  const isRunning = useServerStore((s) => s.isRunning());
  const agentTreesByProject = useServerStore((s) => s.agentTreesByProject);
  const selectedProjectRoot = useSessionStore((s) => s.selectedProjectRoot);

  const queuedMessages = useInteractionStore((s) => s.queuedMessages);
  const pendingPlan = useInteractionStore((s) => s.pendingPlan);
  const pendingPlanAgentId = useInteractionStore((s) => s.pendingPlanAgentId);
  const pendingAskUser = useInteractionStore((s) => s.pendingAskUser);
  const activePlan = useInteractionStore((s) => s.activePlan);
  const verboseMode = useUiStore((s) => s.verboseMode);
  const overlay = useUiStore((s) => s.overlay);
  const modelPickerOpen = useUiStore((s) => s.modelPickerOpen);

  // --- Auto-scroll ---
  const lastMsg = chatMessages[chatMessages.length - 1];
  const { chatEndRef, scrollToBottom, showScrollButton } = useAutoScroll(chatMessages, lastMsg);

  // --- Run info ---
  const { runningMainRunIds } = useRunInfo();

  // --- Derived ---
  const effectiveRoot = projectRoot || selectedProjectRoot;

  // --- Chat actions ---
  const {
    sendChatMessage,
    respondToAskUser,
    approvePlan,
    rejectPlan,
    editPlan,
  } = useChatActions(scrollToBottom, runningMainRunIds, effectiveRoot);

  // --- Transport (WebRTC) ---
  const effectiveSessionId = sessionId || null;
  useTransport({
    sessionId: effectiveSessionId,
    onParseError: () => {
      useChatStore.getState().fetchSessionState();
      useServerStore.getState().fetchAgentRuns();
    },
  });

  const mainAgentIds = useMemo(() => agents.map((a) => a.name.toLowerCase()), [agents]);
  const agentTree = useMemo(
    () => agentTreesByProject[effectiveRoot] || {},
    [agentTreesByProject, effectiveRoot],
  );
  const subagents = useMemo(
    () => buildSubagentInfos(agentTree, mainAgentIds),
    [agentTree, mainAgentIds],
  );

  // --- Model switcher ---
  const switchModel = useCallback(async (modelId: string) => {
    try {
      const resp = await fetch('/api/config');
      if (resp.ok) {
        const config = await resp.json();
        const newDefaults = [modelId];
        const updated = { ...config, routing: { ...config.routing, default_models: newDefaults } };
        const saveResp = await fetch('/api/config', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(updated) });
        if (saveResp.ok) {
          useServerStore.setState({ defaultModels: newDefaults });
          useUiStore.getState().setModelPickerOpen(false);
          useUiStore.getState().setOverlay(`Switched to: \`${modelId}\``);
        }
      }
    } catch (e) {
      useUiStore.getState().setOverlay(`Error switching model: ${e}`);
      useUiStore.getState().setModelPickerOpen(false);
    }
  }, []);

  return (
    <ChatPanel
      chatMessages={displayMessages}
      queuedMessages={queuedMessages}
      chatEndRef={chatEndRef}
      projectRoot={effectiveRoot}
      sessionId={effectiveSessionId}
      selectedAgent={selectedAgent}
      setSelectedAgent={useServerStore.getState().setSelectedAgent}
      skills={skills}
      agents={agents}
      mainAgents={agents}
      subagents={subagents}
      runningMainRunIds={runningMainRunIds}
      cancellingRunIds={cancellingRunIds}
      onCancelRun={(id) => useServerStore.getState().cancelAgentRun(id)}
      onCancelAgentRun={(id) => useServerStore.getState().cancelAgentRun(id)}
      isRunning={isRunning}
      onSendMessage={sendChatMessage}
      activePlan={activePlan}
      pendingPlan={pendingPlan}
      pendingPlanAgentId={pendingPlanAgentId}
      agentContext={agentContext}
      onApprovePlan={approvePlan}
      onRejectPlan={rejectPlan}
      onEditPlan={editPlan}
      pendingAskUser={pendingAskUser}
      onRespondToAskUser={respondToAskUser}
      verboseMode={verboseMode}
      agentStatus={agentStatus}
      overlay={overlay}
      onDismissOverlay={() => { useUiStore.getState().setOverlay(null); useUiStore.getState().setModelPickerOpen(false); }}
      modelPickerOpen={modelPickerOpen}
      models={models}
      defaultModels={defaultModels}
      tokensPerSec={tokensPerSec}
      onSwitchModel={switchModel}
      mobile={mode === 'mobile'}
      scrollToBottom={scrollToBottom}
      showScrollButton={showScrollButton}
    />
  );
};

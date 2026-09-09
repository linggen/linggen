import type { UiEvent } from '../../types';
import { useChatStore } from '../../stores/chatStore';
import { useSessionStore } from '../../stores/sessionStore';
import { useServerStore } from '../../stores/serverStore';
import { useInteractionStore } from '../../stores/interactionStore';
import { useUiStore } from '../../stores/uiStore';
import { getSessionId } from './_shared';

// ---------------------------------------------------------------------------
// AskUser — model is asking the user a structured question
// ---------------------------------------------------------------------------

export function handleAskUser(item: UiEvent): void {
  const { question_id, questions } = item.data || {};
  if (!question_id || !questions) return;

  // Only show the prompt when it belongs to the UI's currently-displayed
  // session. Without this filter, a permission prompt raised by the memory
  // skill's session leaks into unrelated user-chat windows that happen to be
  // open in another tab, because `pendingAskUser` is a single global slot.
  const evtSid = item.session_id ? String(item.session_id) : null;
  const activeSid = useSessionStore.getState().activeSessionId;
  if (evtSid && activeSid && evtSid !== activeSid) return;

  useInteractionStore.getState().setPendingAskUser({
    questionId: question_id,
    agentId: String(item.agent_id || ''),
    questions,
  });
}

// ---------------------------------------------------------------------------
// Widget resolved — dismiss interactive widgets (permission prompts, etc.)
// ---------------------------------------------------------------------------

export function handleWidgetResolved(item: UiEvent): void {
  const widgetId = item.data?.widget_id as string | undefined;
  if (!widgetId) return;
  const interaction = useInteractionStore.getState();
  // Dismiss AskUser permission widget
  if (interaction.pendingAskUser?.questionId === widgetId) {
    interaction.setPendingAskUser(null);
  }
  // Dismiss plan widget (defensive — plan normally syncs via PlanUpdate)
  if (interaction.pendingPlanAgentId === widgetId) {
    interaction.setPendingPlan(null);
    interaction.setPendingPlanAgentId(null);
  }
}

// ---------------------------------------------------------------------------
// Queue — pending-user-message queue for a busy agent
// ---------------------------------------------------------------------------

export function handleQueue(item: UiEvent): void {
  const { activeSessionId } = useSessionStore.getState();
  const session = activeSessionId || 'default';
  if (item.session_id !== session) return;
  const items = Array.isArray(item.data?.items) ? item.data.items : [];
  useInteractionStore.getState().setQueuedMessages(items);
}

// ---------------------------------------------------------------------------
// Model fallback — preferred model failed, fell back to another
// ---------------------------------------------------------------------------

export function handleModelFallback(item: UiEvent): void {
  const text = String(item.text || 'Model switched');
  useChatStore.getState().addMessage({
    role: 'agent' as const,
    from: 'system',
    to: '',
    text: `\u26A0\uFE0F ${text}`,
    timestamp: new Date().toLocaleTimeString(),
    timestampMs: Date.now(),
    isGenerating: false,
  });
  const sid = getSessionId(item);
  if (!sid) return;
  useServerStore.getState().setAgentStatusText((prev) => ({
    ...prev,
    [sid]: `Fallback: ${item.data?.actual_model || 'alternate model'}`,
  }));
  // Move the picker onto the model that is actually running. The server has
  // already pinned it to the session; a picker still naming the model that
  // just refused is showing state the server no longer holds, and the user
  // cannot undo a switch they cannot see.
  const actual = item.data?.actual_model;
  if (typeof actual === 'string' && actual && sid === useSessionStore.getState().activeSessionId) {
    useUiStore.getState().setSessionModel(actual);
  }
}

import type { UiEventOf } from '../../types';
import { useChatStore } from '../../stores/chatStore';
import { useSessionStore } from '../../stores/sessionStore';
import { useServerStore } from '../../stores/serverStore';
import { useInteractionStore } from '../../stores/interactionStore';
import { useUiStore } from '../../stores/uiStore';
import { getSessionId } from './_shared';
import { withAgentQueue } from '../queuedList.mts';
import { chatAgentOf, withMemberModel } from '../sessionMembers.mts';
import { agentTracker } from '../agentTracker';

// ---------------------------------------------------------------------------
// AskUser — model is asking the user a structured question
// ---------------------------------------------------------------------------

export function handleAskUser(item: UiEventOf<'ask_user'>): void {
  const { question_id, questions } = item.data || {};
  if (!question_id || !questions) return;

  const evtSid = item.session_id ? String(item.session_id) : null;
  if (!evtSid) return;

  useInteractionStore.getState().enqueueAsk(evtSid, {
    questionId: question_id,
    agentId: String(item.agent_id || ''),
    questions,
  });
}

// ---------------------------------------------------------------------------
// Widget resolved — dismiss interactive widgets (permission prompts, etc.)
// ---------------------------------------------------------------------------

export function handleWidgetResolved(item: UiEventOf<'widget_resolved'>): void {
  const widgetId = item.data?.widget_id as string | undefined;
  if (!widgetId) return;
  const interaction = useInteractionStore.getState();
  // A resolution only removes the matching question in its session.
  if (item.session_id) interaction.removeAsk(String(item.session_id), widgetId);
  // Dismiss plan widget (defensive — plan normally syncs via PlanUpdate)
  if (interaction.pendingPlanAgentId === widgetId) {
    interaction.setPendingPlan(null);
    interaction.setPendingPlanAgentId(null);
  }
}

// ---------------------------------------------------------------------------
// Queue — pending-user-message queue for a busy agent
// ---------------------------------------------------------------------------

export function handleQueue(item: UiEventOf<'queue'>): void {
  const { activeSessionId } = useSessionStore.getState();
  const session = activeSessionId || 'default';
  if (item.session_id !== session) return;
  const items = Array.isArray(item.data?.items) ? item.data.items : [];
  // The event is one agent's queue: the session's other agents keep theirs.
  const agent = item.agent_id ?? '';
  useInteractionStore.getState().setQueuedMessages((list) => withAgentQueue(list, agent, items));
}

/** A session's background task list, whole, after any change. */
export function handleTasks(item: UiEventOf<'tasks'>): void {
  if (item.session_id !== useSessionStore.getState().activeSessionId) return;
  useInteractionStore.getState().setBackgroundTasks(Array.isArray(item.data?.tasks) ? item.data.tasks : []);
}

// ---------------------------------------------------------------------------
// Model fallback — preferred model failed, fell back to another
// ---------------------------------------------------------------------------

export function handleModelFallback(item: UiEventOf<'model_fallback'>): void {
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
  const actual = item.data?.actual_model;
  const agent = (item.agent_id ?? '').toLowerCase();
  if (typeof actual !== 'string' || !actual || !agent || agentTracker.getParent(agent)) return;
  const sessions = useSessionStore.getState();
  const update = (session: typeof sessions.allSessions[number]) => session.id === sid
    ? { ...session, agents: withMemberModel(session.agents, agent, actual) }
    : session;
  useSessionStore.setState({ allSessions: sessions.allSessions.map(update), sessions: sessions.sessions.map(update) });
  if (sid !== sessions.activeSessionId) return;
  const ui = useUiStore.getState();
  const selected = chatAgentOf(useServerStore.getState().selectedAgent, ui.sessionMembers);
  ui.setSessionMembers(withMemberModel(ui.sessionMembers, agent, actual));
  if (selected === agent) ui.setSessionModel(actual);
}

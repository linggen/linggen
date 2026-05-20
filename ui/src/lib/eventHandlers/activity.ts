import type { UiEvent, SubagentToolStep } from '../../types';
import { useChatStore } from '../../stores/chatStore';
import { useServerStore } from '../../stores/serverStore';
import { useInteractionStore } from '../../stores/interactionStore';
import type { AgentStatusValue } from '../../stores/serverStore';
import { agentTracker } from '../agentTracker';
import { normalizeAgentStatus } from '../messageUtils';
import { getSessionId, toolPrefixMap } from './_shared';

export function handleActivity(item: UiEvent): void {
  const agentId = String(item.agent_id || '');
  if (!agentId) return;
  const statusRaw = String(item.data?.status || '').trim();
  const nextStatus = normalizeAgentStatus(statusRaw) as AgentStatusValue;
  const statusText = String(item.text || '').trim();

  // Route subagent activity to parent tree. Prefer the emitter's unique
  // run_id as the tracking key — distinguishes parallel subagents that
  // share the same agent_id (e.g. six "ling" subagents running in parallel).
  //
  // The wire field name varies per event type: SubagentSpawned/Result use
  // `parent_id`; AgentStatus uses `parent_agent_id` (see server/events.rs).
  // Accept both so the first subagent status event arriving BEFORE
  // SubagentSpawned has registered the tracker still routes to the
  // subagent tree instead of leaking into the session-level status —
  // root cause of the "Idle spinner stuck after encoder run" bug.
  const runIdFromData = item.data?.run_id ? String(item.data.run_id) : null;
  const trackingId = runIdFromData || agentId;
  const parentIdFromData =
    (item.data?.parent_agent_id ? String(item.data.parent_agent_id) : null) ||
    (item.data?.parent_id ? String(item.data.parent_id) : null);
  const parentIdForSubagent =
    agentTracker.getParent(trackingId) ||
    (parentIdFromData ? parentIdFromData.toLowerCase() : null);

  if (parentIdForSubagent && statusRaw !== 'mission_triggered') {
    applySubagentActivity({
      parentId: parentIdForSubagent,
      agentId: trackingId,
      phase: item.phase,
      nextStatus,
      statusText,
    });
    return;
  }

  applyTopLevelActivity({
    agentId,
    phase: item.phase,
    nextStatus,
    statusRaw,
    statusText,
    sid: getSessionId(item),
  });
}

function applySubagentActivity(opts: {
  parentId: string;
  agentId: string;
  phase: string | undefined;
  nextStatus: AgentStatusValue;
  statusText: string;
}): void {
  const { parentId, agentId, phase, nextStatus, statusText } = opts;
  const chatStore = useChatStore.getState();

  if (nextStatus === 'calling_tool') {
    if (phase !== 'done') {
      const toolCount = agentTracker.incrementToolCount(agentId);
      const newStep = parseToolStep(statusText);
      chatStore.updateSubagentTree(parentId, agentId,
        (entry) => ({
          ...entry,
          toolCount: toolCount || entry.toolCount + 1,
          currentActivity: statusText || entry.currentActivity,
          toolSteps: newStep ? [...(entry.toolSteps || []), newStep] : (entry.toolSteps || []),
        }));
    } else {
      const isFailed = statusText.toLowerCase().includes('failed');
      chatStore.updateSubagentTree(parentId, agentId,
        (entry) => {
          const steps = [...(entry.toolSteps || [])];
          if (steps.length > 0) {
            steps[steps.length - 1] = { ...steps[steps.length - 1], status: isFailed ? 'failed' : 'done' };
          }
          return { ...entry, toolSteps: steps };
        });
    }
    return;
  }

  if (nextStatus === 'thinking' || nextStatus === 'model_loading') {
    chatStore.updateSubagentTree(parentId, agentId,
      (entry) => ({
        ...entry,
        currentActivity: statusText || (nextStatus === 'thinking' ? 'Thinking...' : 'Model loading...'),
      }));
    return;
  }

  if (nextStatus === 'idle') {
    chatStore.updateSubagentTree(parentId, agentId,
      (entry) => ({ ...entry, currentActivity: null }));
  }
}

function parseToolStep(statusText: string): SubagentToolStep | null {
  if (!statusText) return null;
  for (const [prefix, toolName] of toolPrefixMap) {
    if (statusText.startsWith(prefix)) {
      return { toolName, args: statusText.slice(prefix.length), status: 'running' };
    }
  }
  return null;
}

function applyTopLevelActivity(opts: {
  agentId: string;
  phase: string | undefined;
  nextStatus: AgentStatusValue;
  statusRaw: string;
  statusText: string;
  sid: string;
}): void {
  const { agentId, phase, nextStatus, statusRaw, statusText, sid } = opts;
  const agentStore = useServerStore.getState();
  const chatStore = useChatStore.getState();

  if (statusRaw && sid && (phase !== 'done' || nextStatus === 'idle')) {
    agentStore.setAgentStatus((prev) => ({ ...prev, [sid]: nextStatus }));
    agentStore.setAgentStatusText((prev) => ({
      ...prev,
      [sid]: resolveStatusText(nextStatus, statusText),
    }));
  }

  if (sid) agentTracker.ensureRunStarted(sid);

  if (statusText.length > 0 && phase !== 'done') {
    if (nextStatus === 'model_loading' || nextStatus === 'thinking') {
      chatStore.setPlaceholder(agentId, statusText);
    } else {
      chatStore.appendActivityWithSegments(agentId, statusText);
    }
  } else if ((nextStatus === 'model_loading' || nextStatus === 'thinking') && phase !== 'done') {
    const placeholder = nextStatus === 'model_loading' ? 'Model loading...' : 'Thinking...';
    chatStore.setPlaceholder(agentId, placeholder);
  }

  if (nextStatus === 'idle' || phase === 'failed') {
    const { elapsed, contextTokens: ctxTokens } = sid ? agentTracker.clearRun(sid) : {};
    chatStore.finalizeOnIdle(agentId, elapsed, ctxTokens);
    useInteractionStore.getState().setActivePlan(null);
  }
}

function resolveStatusText(nextStatus: AgentStatusValue, statusText: string): string {
  if (nextStatus === 'idle') return 'Idle';
  if (statusText.length > 0) return statusText;
  switch (nextStatus) {
    case 'calling_tool': return 'Calling Tool';
    case 'model_loading': return 'Model Loading';
    case 'thinking': return 'Thinking';
    case 'working': return 'Working';
    default: return 'Idle';
  }
}

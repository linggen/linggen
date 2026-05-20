import type { UiEvent, SubagentTreeEntry, Plan } from '../../types';
import { useChatStore } from '../../stores/chatStore';
import { useServerStore } from '../../stores/serverStore';
import { useInteractionStore } from '../../stores/interactionStore';
import { agentTracker } from '../agentTracker';
import { getSessionId } from './_shared';
import type { AgentStatusValue } from '../../stores/serverStore';

export function handleRun(item: UiEvent): void {
  switch (item.phase) {
    case 'sync':
    case 'resync':
      // sync/resync are now handled by server-pushed page_state — no HTTP fetches needed.
      return;
    case 'outcome':
      handleRunOutcome();
      return;
    case 'context_usage':
      handleContextUsage(item);
      return;
    case 'plan_update':
      handlePlanUpdate(item);
      return;
    case 'subagent_spawned':
      handleSubagentSpawned(item);
      return;
    case 'subagent_result':
      handleSubagentResult(item);
      return;
  }
}

function handleRunOutcome(): void {
  // Skip session state fetch for consumer mode — messages arrive via streaming
  // events, and the HTTP fetch goes through the WebRTC tunnel which blocks
  // /api/workspace/state for browser consumers.
  useChatStore.getState().fetchSessionState();
  useServerStore.setState({ tokensPerSec: 0 });
}

function handleContextUsage(item: UiEvent): void {
  if (!item.data) return;
  const agentIdKey =
    typeof item.data.agent_id === 'string'
      ? item.data.agent_id.toLowerCase()
      : (item.agent_id || '').toLowerCase();
  if (!agentIdKey) return;

  const sid = getSessionId(item);
  const estTokens = Number(item.data.estimated_tokens || 0);
  if (sid) agentTracker.latestContextTokens[sid] = estTokens;

  // Accumulate session token usage from actual_prompt/completion_tokens
  const promptDelta = Number(item.data.actual_prompt_tokens || 0);
  const completionDelta = Number(item.data.actual_completion_tokens || 0);
  if (promptDelta > 0 || completionDelta > 0) {
    const prev = useServerStore.getState().sessionTokens;
    useServerStore.setState({
      sessionTokens: {
        prompt: prev.prompt + promptDelta,
        completion: prev.completion + completionDelta,
      },
    });
  }

  const parentId = agentTracker.getParent(agentIdKey);
  if (parentId) {
    agentTracker.setSubagentContextTokens(agentIdKey, estTokens);
    useChatStore.getState().updateSubagentTree(parentId, agentIdKey,
      (entry) => ({ ...entry, contextTokens: estTokens }));
    return;
  }

  if (sid) {
    useServerStore.getState().setAgentContext((prev) => ({
      ...prev,
      [sid]: {
        tokens: estTokens,
        messages: Number(item.data.message_count || 0),
        tokenLimit:
          typeof item.data.token_limit === 'number'
            ? Number(item.data.token_limit)
            : prev[sid]?.tokenLimit,
      },
    }));
  }
}

function handlePlanUpdate(item: UiEvent): void {
  if (!item.data?.plan) return;
  const plan = item.data.plan as Plan;
  const rawId = String(item.agent_id || '');
  const match = rawId.match(/^run-(.+?)-\d+/);
  const agentId = match ? match[1] : rawId;

  const interaction = useInteractionStore.getState();
  interaction.setActivePlan(plan);
  if (plan.status === 'planned') {
    interaction.setPendingPlan(plan);
    interaction.setPendingPlanAgentId(agentId);
  } else {
    // approved, executing, completed, rejected — no longer waiting for user decision
    interaction.setPendingPlan(null);
    interaction.setPendingPlanAgentId(null);
  }
  const planText = JSON.stringify({ type: 'plan', plan });
  useChatStore.getState().upsertPlan(agentId, planText);
}

function handleSubagentSpawned(item: UiEvent): void {
  if (!item.data) return;
  const parentId = String(item.agent_id || '').toLowerCase();
  // Prefer the unique run_id (distinguishes parallel subagents that share
  // the same agent_id, e.g. six "ling" subagents). Fall back to subagent_id
  // (agent spec name) for back-compat with older events.
  const agentName = String(item.data.subagent_id || '');
  const trackingId = String(item.data.subagent_run_id || item.data.subagent_id || '');
  const task = String(item.data.task || '');
  if (!trackingId || !parentId) return;

  agentTracker.registerSubagent(trackingId, parentId);
  const newEntry: SubagentTreeEntry = {
    subagentId: trackingId,
    agentName: agentName || trackingId,
    task,
    status: 'running',
    toolCount: 0,
    contextTokens: 0,
    currentActivity: null,
    toolSteps: [],
    timestampMs: Date.now(),
  };
  useChatStore.getState().addSubagentToTree(parentId, newEntry);
}

function handleSubagentResult(item: UiEvent): void {
  if (!item.data) return;
  const parentId = String(item.agent_id || '').toLowerCase();
  const trackingId = String(item.data.subagent_run_id || item.data.subagent_id || '');
  if (!trackingId || !parentId) return;

  // The SubagentResult event also carries the subagent's outcome text;
  // capture it as a fallback for the expand-on-click view in case the
  // terminal Message event was missed or arrived after unregister.
  const outcomeText = extractOutcomeText(item.data.outcome);

  useChatStore.getState().updateSubagentTree(parentId, trackingId,
    (entry) => ({
      ...entry,
      status: 'done',
      currentActivity: null,
      // Preserve any resultText already captured by handleMessage —
      // outcome is the fallback, not an overwrite.
      resultText: entry.resultText ?? outcomeText ?? entry.resultText,
    }));

  // Defer the unregister so a trailing Message event (the subagent's
  // final terminal text, which can be dispatched a few ms after
  // SubagentResult depending on async-channel ordering) still finds
  // this parent mapping and routes into the subagent widget instead of
  // the main chat. 2s is comfortably longer than any normal event-bus
  // lag without keeping the entry alive long enough to confuse a future
  // subagent that reuses the same run_id.
  setTimeout(() => agentTracker.unregisterSubagent(trackingId), 2000);

  // Defense for the stuck-Idle-spinner case: a subagent status event
  // that arrived BEFORE its SubagentSpawned could pollute the
  // session-level `agentStatus[sid] = 'thinking'`. Subagent-routed
  // events never reset that. So when this subagent finishes, clear the
  // session status back to idle. Safe because the parent's own turn
  // has already emitted its TurnComplete by the time the encoder /
  // dream subagent gets here — there's no other foreground agent.
  const sid = getSessionId(item);
  if (sid) {
    const agentStore = useServerStore.getState();
    agentStore.setAgentStatus((prev) => ({ ...prev, [sid]: 'idle' as AgentStatusValue }));
    agentStore.setAgentStatusText((prev) => ({ ...prev, [sid]: 'Idle' }));
  }
}

function extractOutcomeText(outcome: unknown): string | undefined {
  if (!outcome) return undefined;
  if (typeof outcome === 'string') return outcome;
  if (typeof outcome === 'object' && outcome !== null) {
    const o = outcome as Record<string, unknown>;
    const candidate = o.text || o.message || o.summary || o.result;
    if (typeof candidate === 'string') return candidate;
  }
  return undefined;
}

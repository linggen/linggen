/**
 * Per-kind event handler functions — dispatches UiEvent events to stores.
 * Transport-agnostic: works with both SSE and WebRTC transports.
 * Reads/writes state directly via Zustand stores — no deps object needed.
 */
import type { UiEvent, ContentBlock, SubagentTreeEntry, SubagentToolStep, Plan } from '../types';
import { useProjectStore } from '../stores/projectStore';
import { useAgentStore } from '../stores/agentStore';
import { useChatStore } from '../stores/chatStore';
import { useUiStore } from '../stores/uiStore';
import { openAppInNewTab } from './appLauncher';
import type { AgentStatusValue } from '../stores/agentStore';
import {
  stripEmbeddedStructuredJson,
  isStatusLineText,
  normalizeAgentStatus,
  shouldHideInternalChatMessage,
} from './messageUtils';

// ---------------------------------------------------------------------------
// Tool activity text parser (mirrors TUI parse_activity_text)
// ---------------------------------------------------------------------------

const toolPrefixMap: [string, string][] = [
  ['Reading file: ', 'Read'],
  ['Read file: ', 'Read'],
  ['Read failed: ', 'Read'],
  ['Writing file: ', 'Write'],
  ['Wrote file: ', 'Write'],
  ['Write failed: ', 'Write'],
  ['Editing file: ', 'Edit'],
  ['Edited file: ', 'Edit'],
  ['Edit failed: ', 'Edit'],
  ['Running command: ', 'Bash'],
  ['Ran command: ', 'Bash'],
  ['Command failed: ', 'Bash'],
  ['Searching: ', 'Grep'],
  ['Searched: ', 'Grep'],
  ['Search failed: ', 'Grep'],
  ['Listing files: ', 'Glob'],
  ['Listed files: ', 'Glob'],
  ['List files failed: ', 'Glob'],
  ['Delegating to subagent: ', 'Task'],
  ['Delegated to subagent: ', 'Task'],
  ['Delegation failed: ', 'Task'],
  ['Fetching URL: ', 'WebFetch'],
  ['Fetched URL: ', 'WebFetch'],
  ['Fetch failed: ', 'WebFetch'],
  ['Searching web: ', 'WebSearch'],
  ['Searched web: ', 'WebSearch'],
  ['Web search failed: ', 'WebSearch'],
  ['Calling tool: ', 'Tool'],
  ['Used tool: ', 'Tool'],
  ['Tool failed: ', 'Tool'],
];

/** Build a user-facing status line for a tool start event. */
function formatToolStartLine(toolName: string, argsStr: string): string {
  try {
    const args = JSON.parse(argsStr);
    switch (toolName) {
      case 'Read': return `Reading file: ${args.file_path || args.path || argsStr}`;
      case 'Write': return `Writing file: ${args.file_path || args.path || argsStr}`;
      case 'Edit': return `Editing file: ${args.file_path || args.path || argsStr}`;
      case 'Bash': {
        const cmd = args.command || args.cmd || '';
        return `Running command: ${cmd.length > 80 ? cmd.slice(0, 77) + '...' : cmd}`;
      }
      case 'Grep': return `Searching: ${args.pattern || argsStr}`;
      case 'Glob': return `Listing files: ${args.pattern || argsStr}`;
      case 'Task':
      case 'delegate_to_agent':
        return `Delegating to subagent: ${args.agent_id || args.agent || argsStr}`;
      case 'WebFetch': return `Fetching URL: ${args.url || argsStr}`;
      case 'WebSearch': return `Searching web: ${args.query || argsStr}`;
      default: return `Calling tool: ${toolName}`;
    }
  } catch {
    return `Calling tool: ${toolName}`;
  }
}

// ---------------------------------------------------------------------------
// Main dispatcher
// ---------------------------------------------------------------------------

export function dispatchEvent(item: UiEvent, sessionIdOverride?: string): void {
  const effectiveSessionId = sessionIdOverride ?? useProjectStore.getState().activeSessionId;
  // Allow notifications and permission prompts through regardless — they are global events.
  if (item.kind !== 'notification' && item.kind !== 'ask_user' && item.session_id && item.session_id !== 'global') {
    // Drop events from other sessions when we have an active session.
    if (effectiveSessionId && item.session_id !== effectiveSessionId) return;
    // Drop session-scoped events when no session is active — they belong to
    // skill apps or other scoped sessions, not the main view.
    if (!effectiveSessionId) return;
  }

  // Skill app bridge: forward key events to parent when embedded as iframe
  if (window.parent !== window) {
    if (item.kind === 'token' && item.text) {
      window.parent.postMessage({ type: 'linggen-skill-event', event: 'stream_token', payload: { text: item.text, done: item.phase === 'done' } }, '*');
    } else if (item.kind === 'turn_complete') {
      const msgs = useChatStore.getState().messages;
      const lastMsg = [...msgs].reverse().find(m => m.role === 'assistant' || (m as any).role === 'agent');
      window.parent.postMessage({ type: 'linggen-skill-event', event: 'stream_end', payload: { text: lastMsg?.text || '' } }, '*');
    }
  }

  switch (item.kind) {
    case 'run':          handleRun(item); return;
    case 'queue':        handleQueue(item); return;
    case 'ask_user':     handleAskUser(item); return;
    case 'text_segment': handleTextSegment(item); return;
    case 'activity':     handleActivity(item); return;
    case 'token':        handleToken(item); return;
    case 'message':      handleMessage(item); return;
    case 'model_fallback': handleModelFallback(item); return;
    case 'content_block':  handleContentBlock(item); return;
    case 'turn_complete':   handleTurnComplete(item); return;
    case 'tool_progress': handleToolProgress(item); return;
    case 'app_launched':   handleAppLaunched(item); return;
    case 'notification':   handleNotification(item); return;
  }
}

// ---------------------------------------------------------------------------
// Per-kind handlers
// ---------------------------------------------------------------------------

function handleRun(item: UiEvent): void {
  const projectStore = useProjectStore.getState();
  const agentStore = useAgentStore.getState();
  const chatStore = useChatStore.getState();

  if (item.phase === 'sync' || item.phase === 'outcome' || item.phase === 'resync') {
    chatStore.fetchWorkspaceState();
    projectStore.fetchFiles(projectStore.currentPath);
    projectStore.fetchAllAgentTrees();
    agentStore.fetchAgentRuns();
    projectStore.fetchSessions();
    return;
  }

  if (item.phase === 'context_usage' && item.data) {
    const agentIdKey =
      typeof item.data.agent_id === 'string'
        ? item.data.agent_id.toLowerCase()
        : (item.agent_id || '').toLowerCase();
    if (!agentIdKey) return;

    const estTokens = Number(item.data.estimated_tokens || 0);
    agentStore._latestContextTokens[agentIdKey] = estTokens;

    const parentId = agentStore._subagentParentMap[agentIdKey];
    if (parentId) {
      const stats = agentStore._subagentStats[agentIdKey];
      if (stats) stats.contextTokens = estTokens;
      chatStore.updateSubagentTree(parentId, agentIdKey,
        (entry) => ({ ...entry, contextTokens: estTokens }));
    } else {
      agentStore.setAgentContext((prev) => ({
        ...prev,
        [agentIdKey]: {
          tokens: estTokens,
          messages: Number(item.data.message_count || 0),
          tokenLimit:
            typeof item.data.token_limit === 'number'
              ? Number(item.data.token_limit)
              : prev[agentIdKey]?.tokenLimit,
        },
      }));
    }
    return;
  }

  if (item.phase === 'plan_update' && item.data?.plan) {
    const plan = item.data.plan as Plan;
    const rawId = String(item.agent_id || '');
    const match = rawId.match(/^run-(.+?)-\d+/);
    const agentId = match ? match[1] : rawId;
    const uiStore = useUiStore.getState();
    uiStore.setActivePlan(plan);
    if (plan.status === 'planned') {
      uiStore.setPendingPlan(plan);
      uiStore.setPendingPlanAgentId(agentId);
    } else {
      // approved, executing, completed, rejected — no longer waiting for user decision
      uiStore.setPendingPlan(null);
      uiStore.setPendingPlanAgentId(null);
    }
    const planText = JSON.stringify({ type: 'plan', plan });
    chatStore.upsertPlan(agentId, planText);
    return;
  }

  if (item.phase === 'subagent_spawned' && item.data) {
    const parentId = String(item.agent_id || '').toLowerCase();
    const subagentId = String(item.data.subagent_id || '');
    const task = String(item.data.task || '');
    if (subagentId && parentId) {
      agentStore._subagentParentMap[subagentId.toLowerCase()] = parentId;
      agentStore._subagentStats[subagentId.toLowerCase()] = { toolCount: 0, contextTokens: 0 };
      const newEntry: SubagentTreeEntry = {
        subagentId,
        agentName: subagentId,
        task,
        status: 'running',
        toolCount: 0,
        contextTokens: 0,
        currentActivity: null,
        toolSteps: [],
      };
      chatStore.addSubagentToTree(parentId, newEntry);
    }
    return;
  }

  if (item.phase === 'subagent_result' && item.data) {
    const parentId = String(item.agent_id || '').toLowerCase();
    const subagentId = String(item.data.subagent_id || '');
    if (subagentId && parentId) {
      chatStore.updateSubagentTree(parentId, subagentId,
        (entry) => ({ ...entry, status: 'done', currentActivity: null }));
      delete agentStore._subagentParentMap[subagentId.toLowerCase()];
      delete agentStore._subagentStats[subagentId.toLowerCase()];
    }
    return;
  }
}

function handleQueue(item: UiEvent): void {
  const { activeSessionId, selectedProjectRoot } = useProjectStore.getState();
  const session = activeSessionId || 'default';
  if (item.project_root === selectedProjectRoot && item.session_id === session) {
    const items = Array.isArray(item.data?.items) ? item.data.items : [];
    useUiStore.getState().setQueuedMessages(items);
  }
}

function handleAskUser(item: UiEvent): void {
  const { question_id, questions } = item.data || {};
  if (question_id && questions) {
    useUiStore.getState().setPendingAskUser({
      questionId: question_id,
      agentId: String(item.agent_id || ''),
      questions,
    });
  }
}

// Re-export for use by SDK and other consumers
export { handleAskUser };

function handleTextSegment(item: UiEvent): void {
  const agentId = String(item.agent_id || '');
  if (!agentId) return;
  if (useAgentStore.getState()._subagentParentMap[agentId.toLowerCase()]) return;
  const segText = String(item.text || '').trim();
  if (!segText) return;
  useChatStore.getState().addTextSegment(agentId, segText);
}

function handleActivity(item: UiEvent): void {
  const agentId = String(item.agent_id || '');
  if (!agentId) return;
  const statusRaw = String(item.data?.status || '').trim();
  const nextStatus = normalizeAgentStatus(statusRaw) as AgentStatusValue;
  const statusText = String(item.text || '').trim();

  const agentStore = useAgentStore.getState();
  const chatStore = useChatStore.getState();
  const uiStore = useUiStore.getState();

  // Route subagent activity to parent tree
  const parentIdFromData = item.data?.parent_id ? String(item.data.parent_id) : null;
  const parentIdForSubagent =
    agentStore._subagentParentMap[agentId.toLowerCase()] ||
    (parentIdFromData ? parentIdFromData.toLowerCase() : null);

  if (parentIdForSubagent && statusRaw !== 'mission_triggered') {
    if (nextStatus === 'calling_tool') {
      if (item.phase !== 'done') {
        const stats = agentStore._subagentStats[agentId.toLowerCase()];
        if (stats) stats.toolCount += 1;
        const newStep: SubagentToolStep | null = statusText
          ? (() => {
              for (const [prefix, toolName] of toolPrefixMap) {
                if (statusText.startsWith(prefix)) {
                  return { toolName, args: statusText.slice(prefix.length), status: 'running' as const };
                }
              }
              return null;
            })()
          : null;
        chatStore.updateSubagentTree(parentIdForSubagent, agentId,
          (entry) => ({
            ...entry,
            toolCount: (stats?.toolCount ?? entry.toolCount + 1),
            currentActivity: statusText || entry.currentActivity,
            toolSteps: newStep ? [...(entry.toolSteps || []), newStep] : (entry.toolSteps || []),
          }));
      } else {
        const isFailed = statusText.toLowerCase().includes('failed');
        chatStore.updateSubagentTree(parentIdForSubagent, agentId,
          (entry) => {
            const steps = [...(entry.toolSteps || [])];
            if (steps.length > 0) {
              steps[steps.length - 1] = { ...steps[steps.length - 1], status: isFailed ? 'failed' : 'done' };
            }
            return { ...entry, toolSteps: steps };
          });
      }
    } else if (nextStatus === 'thinking' || nextStatus === 'model_loading') {
      chatStore.updateSubagentTree(parentIdForSubagent, agentId,
        (entry) => ({
          ...entry,
          currentActivity: statusText || (nextStatus === 'thinking' ? 'Thinking...' : 'Model loading...'),
        }));
    } else if (nextStatus === 'idle') {
      chatStore.updateSubagentTree(parentIdForSubagent, agentId,
        (entry) => ({ ...entry, currentActivity: null }));
    }
    return;
  }

  if (statusRaw) {
    if (item.phase !== 'done' || nextStatus === 'idle') {
      agentStore.setAgentStatus((prev) => ({ ...prev, [agentId]: nextStatus }));
      agentStore.setAgentStatusText((prev) => ({
        ...prev,
        [agentId]:
          nextStatus === 'idle'
            ? 'Idle'
            : statusText.length > 0
              ? statusText
              : nextStatus === 'calling_tool'
                ? 'Calling Tool'
                : nextStatus === 'model_loading'
                  ? 'Model Loading'
                  : nextStatus === 'thinking'
                    ? 'Thinking'
                    : nextStatus === 'working'
                      ? 'Working'
                      : 'Idle',
      }));
    }
  }

  if (!agentStore._runStartTs[agentId]) {
    agentStore._runStartTs[agentId] = Date.now();
  }

  if (statusText.length > 0 && item.phase !== 'done') {
    if (nextStatus === 'model_loading' || nextStatus === 'thinking') {
      chatStore.setPlaceholder(agentId, statusText);
    } else {
      chatStore.appendActivityWithSegments(agentId, statusText);
    }
  } else if ((nextStatus === 'model_loading' || nextStatus === 'thinking') && item.phase !== 'done') {
    const placeholder = nextStatus === 'model_loading' ? 'Model loading...' : 'Thinking...';
    chatStore.setPlaceholder(agentId, placeholder);
  }

  if (nextStatus === 'idle' || item.phase === 'failed') {
    const startTs = agentStore._runStartTs[agentId];
    const elapsed = startTs ? Date.now() - startTs : undefined;
    const ctxTokens = agentStore._latestContextTokens[agentId] || undefined;
    delete agentStore._runStartTs[agentId];
    chatStore.finalizeOnIdle(agentId, elapsed, ctxTokens);
    uiStore.setActivePlan(null);
  }
}

// Token batching: accumulate tokens and flush to React state on rAF
// to avoid "Maximum update depth exceeded" when tokens arrive very fast.
const _tokenBuffer: Map<string, { text: string; isThinking: boolean }> = new Map();
let _tokenFlushScheduled = false;

function flushTokenBuffer() {
  _tokenFlushScheduled = false;
  if (_tokenBuffer.size === 0) return;
  const chatStore = useChatStore.getState();
  for (const [agentId, { text, isThinking }] of _tokenBuffer) {
    if (text) chatStore.appendToken(agentId, text, isThinking);
  }
  _tokenBuffer.clear();
}

function handleToken(item: UiEvent): void {
  const agentId = String(item.agent_id || '');
  const isThinking = item.data?.thinking === true;
  if (!agentId) return;

  const agentStore = useAgentStore.getState();
  if (agentStore._subagentParentMap[agentId.toLowerCase()]) return;

  if (item.phase === 'done') {
    // Flush any pending tokens before marking thinking done
    if (_tokenBuffer.size > 0) flushTokenBuffer();
    if (isThinking) {
      useChatStore.getState().setThinkingFlag(agentId);
    }
    return;
  }

  const tokenText = String(item.text || '');
  const existing = _tokenBuffer.get(agentId);
  if (existing && existing.isThinking === isThinking) {
    existing.text += tokenText;
  } else {
    // Flush if thinking state changed for this agent
    if (existing) flushTokenBuffer();
    _tokenBuffer.set(agentId, { text: tokenText, isThinking });
  }

  if (!_tokenFlushScheduled) {
    _tokenFlushScheduled = true;
    requestAnimationFrame(flushTokenBuffer);
  }
}

function handleMessage(item: UiEvent): void {
  const from = String(item.data?.from || item.agent_id || 'assistant');
  const to = String(item.data?.to || '');
  let content = String(item.text || '');
  if (!content) return;
  if (shouldHideInternalChatMessage(from, content)) return;

  const agentStore = useAgentStore.getState();
  if (agentStore._subagentParentMap[from.toLowerCase()]) return;

  try {
    const parsed = JSON.parse(content);
    if (parsed?.type === 'plan' && parsed?.plan) return;
  } catch (_e) { /* not JSON */ }

  if (from !== 'user') {
    content = stripEmbeddedStructuredJson(content);
    if (!content) return;
  }

  const chatStore = useChatStore.getState();
  if (from !== 'user' && isStatusLineText(content)) {
    chatStore.appendActivity(from, content);
    return;
  }

  const tsMs = Number(item.ts_ms || Date.now());
  const msgStartTs = agentStore._runStartTs[from];
  const msgElapsed = msgStartTs ? Date.now() - msgStartTs : undefined;
  const msgCtxTokens = agentStore._latestContextTokens[from] || undefined;
  delete agentStore._runStartTs[from];

  const isError = from !== 'user' && content.startsWith('Error:');

  chatStore.finalizeMessage(from, content, to, tsMs, msgElapsed, msgCtxTokens, isError || undefined);
}

function handleContentBlock(item: UiEvent): void {
  const agentId = String(item.agent_id || '');
  if (!agentId) return;
  const data = item.data || {};

  const agentStore = useAgentStore.getState();
  const chatStore = useChatStore.getState();

  // Route subagent content blocks to parent tree
  const parentId = agentStore._subagentParentMap[agentId.toLowerCase()];
  if (parentId) {
    if (item.phase === 'start' && data.block_type === 'tool_use') {
      const stats = agentStore._subagentStats[agentId.toLowerCase()];
      if (stats) stats.toolCount += 1;
      const newStep: SubagentToolStep = {
        toolName: data.tool || 'Tool',
        args: data.args || '',
        status: 'running',
      };
      chatStore.updateSubagentTree(parentId, agentId,
        (entry) => ({
          ...entry,
          toolCount: (stats?.toolCount ?? entry.toolCount + 1),
          currentActivity: `${data.tool || 'Tool'}: ${data.args || ''}`,
          toolSteps: [...(entry.toolSteps || []), newStep],
        }));
    } else if (item.phase === 'update') {
      const isFailed = data.status === 'failed';
      chatStore.updateSubagentTree(parentId, agentId,
        (entry) => {
          const steps = [...(entry.toolSteps || [])];
          if (steps.length > 0) {
            steps[steps.length - 1] = { ...steps[steps.length - 1], status: isFailed ? 'failed' : 'done' };
          }
          return { ...entry, toolSteps: steps, currentActivity: data.summary || entry.currentActivity };
        });
    }
    return;
  }

  if (item.phase === 'start') {
    const blockType = String(data.block_type || 'text');
    const block: ContentBlock = {
      type: blockType as ContentBlock['type'],
      id: String(data.block_id || ''),
      tool: data.tool || undefined,
      args: data.args || undefined,
      status: blockType === 'tool_use' ? 'running' : undefined,
      text: blockType === 'text' ? (data.args || '') : undefined,
    };
    chatStore.contentBlockStart(agentId, block);

    if (blockType === 'tool_use') {
      const toolName = data.tool || 'Tool';
      const toolArgs = data.args || '';
      const activityLine = formatToolStartLine(toolName, toolArgs);
      chatStore.appendActivity(agentId, activityLine);

      agentStore.setAgentStatus((prev) => ({ ...prev, [agentId]: 'calling_tool' as AgentStatusValue }));
      agentStore.setAgentStatusText((prev) => ({ ...prev, [agentId]: activityLine }));

      if (!agentStore._runStartTs[agentId]) {
        agentStore._runStartTs[agentId] = Date.now();
      }
    }
  } else if (item.phase === 'update') {
    const diffData = data.diff_type
      ? {
          diff_type: data.diff_type as 'edit' | 'write',
          path: data.path || '',
          old_string: data.old_string,
          new_string: data.new_string,
          new_content: data.new_content,
          start_line: typeof data.start_line === 'number' ? data.start_line : undefined,
          lines_written: typeof data.lines_written === 'number' ? data.lines_written : undefined,
        }
      : undefined;
    chatStore.contentBlockUpdate(
      agentId,
      String(data.block_id || ''),
      (data.status as 'running' | 'done' | 'failed' | undefined) || undefined,
      data.summary || undefined,
      data.is_error ?? undefined,
      diffData,
      Array.isArray(data.bash_output) ? data.bash_output as string[] : undefined,
    );

    if (data.summary) {
      chatStore.appendActivity(agentId, data.summary);
    }
  }
}

function handleTurnComplete(item: UiEvent): void {
  const agentId = String(item.agent_id || '');
  if (!agentId) return;

  const agentStore = useAgentStore.getState();
  if (agentStore._subagentParentMap[agentId.toLowerCase()]) return;

  const data = item.data || {};
  const durationMs = typeof data.duration_ms === 'number' ? data.duration_ms : undefined;
  const contextTokens = typeof data.context_tokens === 'number' ? data.context_tokens : undefined;

  const startTs = agentStore._runStartTs[agentId];
  const elapsed = durationMs || (startTs ? Date.now() - startTs : undefined);
  const ctxTokens = contextTokens || agentStore._latestContextTokens[agentId] || undefined;
  delete agentStore._runStartTs[agentId];

  useChatStore.getState().turnComplete(agentId, elapsed, ctxTokens);
  useUiStore.getState().setPendingAskUser(null);

  // Ensure agent status transitions to idle — the subsequent AgentStatus(idle)
  // event may arrive late or be missed, leaving the spinner stuck on "Thinking…".
  agentStore.setAgentStatus((prev) => ({ ...prev, [agentId]: 'idle' }));
  agentStore.setAgentStatusText((prev) => ({ ...prev, [agentId]: 'Idle' }));
}

function handleToolProgress(item: UiEvent): void {
  const agentId = String(item.agent_id || '');
  if (!agentId) return;
  if (useAgentStore.getState()._subagentParentMap[agentId.toLowerCase()]) return;
  const data = item.data || {};
  const line = String(data.line || item.text || '');
  if (!line) return;
  useChatStore.getState().toolProgress(agentId, line);
}

function handleAppLaunched(item: UiEvent): void {
  const data = item.data || {};
  const url = data.url || '';
  if (url) {
    openAppInNewTab(url);
  }
}

function handleModelFallback(item: UiEvent): void {
  const agentId = String(item.agent_id || '');
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
  if (agentId) {
    useAgentStore.getState().setAgentStatusText((prev) => ({
      ...prev,
      [agentId]: `Fallback: ${item.data?.actual_model || 'alternate model'}`,
    }));
  }
}

function handleNotification(item: UiEvent): void {
  const data = item.data;
  if (!data) return;

  switch (data.kind as string) {
    case 'mission_completed': {
      const name = String(data.mission_name || data.mission_id || 'Mission');
      const status = String(data.status || 'completed');
      const variant = status === 'completed' ? 'success' as const : 'error' as const;
      const label = status === 'completed' ? 'completed' : 'failed';
      useUiStore.getState().addToast({ message: `Mission "${name}" ${label}`, variant });
      useUiStore.getState().bumpMissionRefreshKey();
      return;
    }
    case 'session_created': {
      // Refresh the unified session list so the new session appears with animation
      useProjectStore.getState().fetchAllSessions();
      return;
    }
    default:
      return;
  }
}

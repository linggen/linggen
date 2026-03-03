/**
 * Per-kind SSE event handler functions.
 * Stateless — all side effects are performed through the deps object.
 */
import type { UiSseMessage, ContentBlock, SubagentTreeEntry, SubagentToolStep, Plan, QueuedChatItem, IdlePromptEvent } from '../types';
import type { ChatAction } from '../hooks/useChatMessages';
import type { AgentStatusValue } from '../hooks/useAgentActivity';
import {
  stripEmbeddedStructuredJson,
  isStatusLineText,
  normalizeAgentStatus,
  shouldHideInternalChatMessage,
} from './messageUtils';

// ---------------------------------------------------------------------------
// Deps interface — everything the SSE handlers need from the app
// ---------------------------------------------------------------------------

type StateSetter<T> = (value: T | ((prev: T) => T)) => void;

export interface SseHandlerDeps {
  chatDispatch: (action: ChatAction) => void;

  // From useAgentActivity
  setAgentStatus: StateSetter<Record<string, AgentStatusValue>>;
  setAgentStatusText: StateSetter<Record<string, string>>;
  setAgentContext: StateSetter<Record<string, { tokens: number; messages: number; tokenLimit?: number }>>;
  subagentParentMapRef: { current: Record<string, string> };
  subagentStatsRef: { current: Record<string, { toolCount: number; contextTokens: number }> };
  runStartTsRef: { current: Record<string, number> };
  latestContextTokensRef: { current: Record<string, number> };

  // UI state setters
  setQueuedMessages: StateSetter<QueuedChatItem[]>;
  setPendingAskUser: StateSetter<import('../types').PendingAskUser | null>;
  setActivePlan: StateSetter<Plan | null>;
  setPendingPlan: StateSetter<Plan | null>;
  setPendingPlanAgentId: StateSetter<string | null>;
  setIdlePromptEvents: StateSetter<IdlePromptEvent[]>;

  // Fetch functions
  fetchWorkspaceState: () => void;
  fetchFiles: (path?: string) => void;
  fetchAllAgentTrees: () => void;
  fetchAgentRuns: () => void;

  // Current state values (read-only snapshots)
  currentPath: string;
  selectedProjectRoot: string;
  activeSessionId: string | null;
}

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

/** Build a user-facing status line for a tool start event (mirrors server tool_status_line). */
function formatToolStartLine(toolName: string, argsStr: string): string {
  // Try to extract a meaningful label from the tool name + args JSON.
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

function parseToolActivity(text: string): { toolName: string; args: string } | null {
  for (const [prefix, toolName] of toolPrefixMap) {
    if (text.startsWith(prefix)) {
      return { toolName, args: text.slice(prefix.length) };
    }
  }
  // Fallback: colon separator
  const colonIdx = text.indexOf(': ');
  if (colonIdx > 0) {
    const label = text.slice(0, colonIdx);
    const args = text.slice(colonIdx + 2);
    return { toolName: label.charAt(0).toUpperCase() + label.slice(1), args };
  }
  return null;
}

// ---------------------------------------------------------------------------
// Main dispatcher
// ---------------------------------------------------------------------------

export function dispatchSseEvent(item: UiSseMessage, deps: SseHandlerDeps): void {
  switch (item.kind) {
    case 'run':          handleRun(item, deps); return;
    case 'queue':        handleQueue(item, deps); return;
    case 'ask_user':     handleAskUser(item, deps); return;
    case 'text_segment': handleTextSegment(item, deps); return;
    case 'activity':     handleActivity(item, deps); return;
    case 'token':        handleToken(item, deps); return;
    case 'message':      handleMessage(item, deps); return;
    case 'model_fallback': handleModelFallback(item, deps); return;
    case 'content_block':  handleContentBlock(item, deps); return;
    case 'turn_complete':   handleTurnComplete(item, deps); return;
    case 'tool_progress': handleToolProgress(item, deps); return;
  }
}

// ---------------------------------------------------------------------------
// Per-kind handlers
// ---------------------------------------------------------------------------

function handleRun(item: UiSseMessage, deps: SseHandlerDeps): void {
  if (item.phase === 'sync' || item.phase === 'outcome' || item.phase === 'resync') {
    deps.fetchWorkspaceState();
    deps.fetchFiles(deps.currentPath);
    deps.fetchAllAgentTrees();
    deps.fetchAgentRuns();
    return;
  }

  if (item.phase === 'context_usage' && item.data) {
    const agentIdKey =
      typeof item.data.agent_id === 'string'
        ? item.data.agent_id.toLowerCase()
        : (item.agent_id || '').toLowerCase();
    if (!agentIdKey) return;

    const estTokens = Number(item.data.estimated_tokens || 0);
    deps.latestContextTokensRef.current[agentIdKey] = estTokens;

    const parentId = deps.subagentParentMapRef.current[agentIdKey];
    if (parentId) {
      const stats = deps.subagentStatsRef.current[agentIdKey];
      if (stats) stats.contextTokens = estTokens;
      deps.chatDispatch({
        type: 'UPDATE_SUBAGENT_TREE', parentId, subagentId: agentIdKey,
        updater: (entry) => ({ ...entry, contextTokens: estTokens }),
      });
    } else {
      deps.setAgentContext((prev) => ({
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
    const agentId = String(item.agent_id || '');
    deps.setActivePlan(plan);
    if (plan.status === 'planned') {
      deps.setPendingPlan(plan);
      deps.setPendingPlanAgentId(agentId);
    }
    const planText = JSON.stringify({ type: 'plan', plan });
    deps.chatDispatch({ type: 'UPSERT_PLAN', agentId, planText });
    return;
  }

  if (item.phase === 'subagent_spawned' && item.data) {
    const parentId = String(item.agent_id || '').toLowerCase();
    const subagentId = String(item.data.subagent_id || '');
    const task = String(item.data.task || '');
    if (subagentId && parentId) {
      deps.subagentParentMapRef.current[subagentId.toLowerCase()] = parentId;
      deps.subagentStatsRef.current[subagentId.toLowerCase()] = { toolCount: 0, contextTokens: 0 };
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
      deps.chatDispatch({ type: 'ADD_SUBAGENT_TO_TREE', parentId, entry: newEntry });
    }
    return;
  }

  if (item.phase === 'subagent_result' && item.data) {
    const parentId = String(item.agent_id || '').toLowerCase();
    const subagentId = String(item.data.subagent_id || '');
    if (subagentId && parentId) {
      deps.chatDispatch({
        type: 'UPDATE_SUBAGENT_TREE', parentId, subagentId,
        updater: (entry) => ({ ...entry, status: 'done', currentActivity: null }),
      });
      delete deps.subagentParentMapRef.current[subagentId.toLowerCase()];
      delete deps.subagentStatsRef.current[subagentId.toLowerCase()];
    }
    return;
  }

}

function handleQueue(item: UiSseMessage, deps: SseHandlerDeps): void {
  const session = deps.activeSessionId || 'default';
  if (item.project_root === deps.selectedProjectRoot && item.session_id === session) {
    const items = Array.isArray(item.data?.items) ? item.data.items : [];
    deps.setQueuedMessages(items);
  }
}

function handleAskUser(item: UiSseMessage, deps: SseHandlerDeps): void {
  const { question_id, questions } = item.data || {};
  if (question_id && questions) {
    deps.setPendingAskUser({
      questionId: question_id,
      agentId: String(item.agent_id || ''),
      questions,
    });
  }
}

function handleTextSegment(item: UiSseMessage, deps: SseHandlerDeps): void {
  const agentId = String(item.agent_id || '');
  if (!agentId) return;
  if (deps.subagentParentMapRef.current[agentId.toLowerCase()]) return;
  const segText = String(item.text || '').trim();
  if (!segText) return;
  deps.chatDispatch({ type: 'ADD_TEXT_SEGMENT', agentId, text: segText });
}

function handleActivity(item: UiSseMessage, deps: SseHandlerDeps): void {
  const agentId = String(item.agent_id || '');
  if (!agentId) return;
  const statusRaw = String(item.data?.status || '').trim();
  const nextStatus = normalizeAgentStatus(statusRaw);
  const statusText = String(item.text || '').trim();

  // Route subagent activity to parent tree
  const parentIdFromData = item.data?.parent_id ? String(item.data.parent_id) : null;
  const parentIdForSubagent =
    deps.subagentParentMapRef.current[agentId.toLowerCase()] ||
    (parentIdFromData ? parentIdFromData.toLowerCase() : null);

  if (parentIdForSubagent && statusRaw !== 'idle_prompt_triggered') {
    if (nextStatus === 'calling_tool') {
      if (item.phase !== 'done') {
        const stats = deps.subagentStatsRef.current[agentId.toLowerCase()];
        if (stats) stats.toolCount += 1;
        const parsed = parseToolActivity(statusText);
        const newStep: SubagentToolStep | null = parsed
          ? { toolName: parsed.toolName, args: parsed.args, status: 'running' }
          : null;
        deps.chatDispatch({
          type: 'UPDATE_SUBAGENT_TREE', parentId: parentIdForSubagent, subagentId: agentId,
          updater: (entry) => ({
            ...entry,
            toolCount: (stats?.toolCount ?? entry.toolCount + 1),
            currentActivity: statusText || entry.currentActivity,
            toolSteps: newStep ? [...(entry.toolSteps || []), newStep] : (entry.toolSteps || []),
          }),
        });
      } else {
        // phase === 'done': mark last tool step as done or failed
        const isFailed = statusText.toLowerCase().includes('failed');
        deps.chatDispatch({
          type: 'UPDATE_SUBAGENT_TREE', parentId: parentIdForSubagent, subagentId: agentId,
          updater: (entry) => {
            const steps = [...(entry.toolSteps || [])];
            if (steps.length > 0) {
              steps[steps.length - 1] = { ...steps[steps.length - 1], status: isFailed ? 'failed' : 'done' };
            }
            return { ...entry, toolSteps: steps };
          },
        });
      }
    } else if (nextStatus === 'thinking' || nextStatus === 'model_loading') {
      deps.chatDispatch({
        type: 'UPDATE_SUBAGENT_TREE', parentId: parentIdForSubagent, subagentId: agentId,
        updater: (entry) => ({
          ...entry,
          currentActivity: statusText || (nextStatus === 'thinking' ? 'Thinking...' : 'Model loading...'),
        }),
      });
    } else if (nextStatus === 'idle') {
      deps.chatDispatch({
        type: 'UPDATE_SUBAGENT_TREE', parentId: parentIdForSubagent, subagentId: agentId,
        updater: (entry) => ({ ...entry, currentActivity: null }),
      });
    }
    return;
  }

  // Capture idle_prompt_triggered events for Mission activity tab
  if (statusRaw === 'idle_prompt_triggered') {
    deps.setIdlePromptEvents((prev) => {
      const evt: IdlePromptEvent = {
        agent_id: agentId,
        project_root: String(item.project_root || ''),
        timestamp: Date.now(),
      };
      const next = [evt, ...prev];
      return next.length > 100 ? next.slice(0, 100) : next;
    });
  }

  if (statusRaw) {
    if (item.phase !== 'done' || nextStatus === 'idle') {
      deps.setAgentStatus((prev) => ({ ...prev, [agentId]: nextStatus }));
      deps.setAgentStatusText((prev) => ({
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

  // Track run start time on first activity
  if (!deps.runStartTsRef.current[agentId]) {
    deps.runStartTsRef.current[agentId] = Date.now();
  }

  // Add tool call status lines as activity entries.
  // Transient statuses (Loading model, Thinking) use SET_PLACEHOLDER so they
  // appear temporarily but don't persist in segments.
  if (statusText.length > 0 && item.phase !== 'done') {
    if (nextStatus === 'model_loading' || nextStatus === 'thinking') {
      deps.chatDispatch({ type: 'SET_PLACEHOLDER', agentId, text: statusText });
    } else {
      deps.chatDispatch({ type: 'APPEND_ACTIVITY_WITH_SEGMENTS', agentId, activityLine: statusText });
    }
  } else if ((nextStatus === 'model_loading' || nextStatus === 'thinking') && item.phase !== 'done') {
    const placeholder = nextStatus === 'model_loading' ? 'Model loading...' : 'Thinking...';
    deps.chatDispatch({ type: 'SET_PLACEHOLDER', agentId, text: placeholder });
  }

  // Finalize on true idle or explicit failure
  if (nextStatus === 'idle' || item.phase === 'failed') {
    const startTs = deps.runStartTsRef.current[agentId];
    const elapsed = startTs ? Date.now() - startTs : undefined;
    const ctxTokens = deps.latestContextTokensRef.current[agentId] || undefined;
    delete deps.runStartTsRef.current[agentId];
    deps.chatDispatch({ type: 'FINALIZE_ON_IDLE', agentId, elapsed, ctxTokens });
  }
}

function handleToken(item: UiSseMessage, deps: SseHandlerDeps): void {
  const agentId = String(item.agent_id || '');
  const isThinking = item.data?.thinking === true;
  if (!agentId) return;

  if (deps.subagentParentMapRef.current[agentId.toLowerCase()]) return;

  if (item.phase === 'done') {
    if (isThinking) {
      deps.chatDispatch({ type: 'SET_THINKING_FLAG', agentId });
    }
    return;
  }

  const tokenText = String(item.text || '');
  deps.chatDispatch({ type: 'APPEND_TOKEN', agentId, tokenText, isThinking });
}

function handleMessage(item: UiSseMessage, deps: SseHandlerDeps): void {
  const from = String(item.data?.from || item.agent_id || 'assistant');
  const to = String(item.data?.to || '');
  let content = String(item.text || '');
  if (!content) return;
  if (shouldHideInternalChatMessage(from, content)) return;

  if (deps.subagentParentMapRef.current[from.toLowerCase()]) return;

  // Skip pure plan JSON messages
  try {
    const parsed = JSON.parse(content);
    if (parsed?.type === 'plan' && parsed?.plan) return;
  } catch (_e) { /* not JSON */ }

  // Strip embedded plan JSON
  if (from !== 'user') {
    content = stripEmbeddedStructuredJson(content);
    if (!content) return;
  }

  if (from !== 'user' && isStatusLineText(content)) {
    deps.chatDispatch({ type: 'APPEND_ACTIVITY', agentId: from, activityLine: content });
    return;
  }

  const tsMs = Number(item.ts_ms || Date.now());
  const msgStartTs = deps.runStartTsRef.current[from];
  const msgElapsed = msgStartTs ? Date.now() - msgStartTs : undefined;
  const msgCtxTokens = deps.latestContextTokensRef.current[from] || undefined;
  delete deps.runStartTsRef.current[from];

  const isError = from !== 'user' && content.startsWith('Error:');

  deps.chatDispatch({
    type: 'FINALIZE_MESSAGE',
    agentId: from, content, to, tsMs,
    elapsed: msgElapsed, ctxTokens: msgCtxTokens,
    isError: isError || undefined,
  });
}

function handleContentBlock(item: UiSseMessage, deps: SseHandlerDeps): void {
  const agentId = String(item.agent_id || '');
  if (!agentId) return;
  const data = item.data || {};

  // Route subagent content blocks to parent tree (as tool steps)
  const parentId = deps.subagentParentMapRef.current[agentId.toLowerCase()];
  if (parentId) {
    if (item.phase === 'start' && data.block_type === 'tool_use') {
      const stats = deps.subagentStatsRef.current[agentId.toLowerCase()];
      if (stats) stats.toolCount += 1;
      const newStep: SubagentToolStep = {
        toolName: data.tool || 'Tool',
        args: data.args || '',
        status: 'running',
      };
      deps.chatDispatch({
        type: 'UPDATE_SUBAGENT_TREE', parentId, subagentId: agentId,
        updater: (entry) => ({
          ...entry,
          toolCount: (stats?.toolCount ?? entry.toolCount + 1),
          currentActivity: `${data.tool || 'Tool'}: ${data.args || ''}`,
          toolSteps: [...(entry.toolSteps || []), newStep],
        }),
      });
    } else if (item.phase === 'update') {
      const isFailed = data.status === 'failed';
      deps.chatDispatch({
        type: 'UPDATE_SUBAGENT_TREE', parentId, subagentId: agentId,
        updater: (entry) => {
          const steps = [...(entry.toolSteps || [])];
          if (steps.length > 0) {
            steps[steps.length - 1] = { ...steps[steps.length - 1], status: isFailed ? 'failed' : 'done' };
          }
          return { ...entry, toolSteps: steps, currentActivity: data.summary || entry.currentActivity };
        },
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
    deps.chatDispatch({ type: 'CONTENT_BLOCK_START', agentId, block });

    // Track activity for tool start (segments are already handled by CONTENT_BLOCK_START).
    if (blockType === 'tool_use') {
      const toolName = data.tool || 'Tool';
      const toolArgs = data.args || '';
      const activityLine = formatToolStartLine(toolName, toolArgs);
      deps.chatDispatch({ type: 'APPEND_ACTIVITY', agentId, activityLine });

      // Update agent status to calling_tool
      deps.setAgentStatus((prev) => ({ ...prev, [agentId]: 'calling_tool' as AgentStatusValue }));
      deps.setAgentStatusText((prev) => ({ ...prev, [agentId]: activityLine }));

      // Track run start time on first tool call
      if (!deps.runStartTsRef.current[agentId]) {
        deps.runStartTsRef.current[agentId] = Date.now();
      }
    }
  } else if (item.phase === 'update') {
    // Extract diff data if present (Edit/Write tools send it via extra).
    const diffData = data.diff_type
      ? {
          diff_type: data.diff_type as 'edit' | 'write',
          path: data.path || '',
          old_string: data.old_string,
          new_string: data.new_string,
          start_line: typeof data.start_line === 'number' ? data.start_line : undefined,
          lines_written: typeof data.lines_written === 'number' ? data.lines_written : undefined,
        }
      : undefined;
    deps.chatDispatch({
      type: 'CONTENT_BLOCK_UPDATE',
      agentId,
      blockId: String(data.block_id || ''),
      status: data.status || undefined,
      summary: data.summary || undefined,
      isError: data.is_error ?? undefined,
      diffData,
      bashOutput: Array.isArray(data.bash_output) ? data.bash_output as string[] : undefined,
    });

    // Track activity for tool completion (segments are already handled by CONTENT_BLOCK_UPDATE).
    if (data.summary) {
      deps.chatDispatch({ type: 'APPEND_ACTIVITY', agentId, activityLine: data.summary });
    }
  }
}

function handleTurnComplete(item: UiSseMessage, deps: SseHandlerDeps): void {
  const agentId = String(item.agent_id || '');
  if (!agentId) return;

  // Skip subagent turn completions
  if (deps.subagentParentMapRef.current[agentId.toLowerCase()]) return;

  const data = item.data || {};
  const durationMs = typeof data.duration_ms === 'number' ? data.duration_ms : undefined;
  const contextTokens = typeof data.context_tokens === 'number' ? data.context_tokens : undefined;

  // Use run-start timing as fallback
  const startTs = deps.runStartTsRef.current[agentId];
  const elapsed = durationMs || (startTs ? Date.now() - startTs : undefined);
  const ctxTokens = contextTokens || deps.latestContextTokensRef.current[agentId] || undefined;
  delete deps.runStartTsRef.current[agentId];

  deps.chatDispatch({ type: 'TURN_COMPLETE', agentId, durationMs: elapsed, contextTokens: ctxTokens });
  // Clear any pending AskUser when the turn ends (e.g. cancelled by new message).
  deps.setPendingAskUser(null);
}

function handleToolProgress(item: UiSseMessage, deps: SseHandlerDeps): void {
  const agentId = String(item.agent_id || '');
  if (!agentId) return;
  // Skip subagent tool progress
  if (deps.subagentParentMapRef.current[agentId.toLowerCase()]) return;
  const data = item.data || {};
  const line = String(data.line || item.text || '');
  if (!line) return;
  deps.chatDispatch({ type: 'TOOL_PROGRESS', agentId, line });
}

function handleModelFallback(item: UiSseMessage, deps: SseHandlerDeps): void {
  const agentId = String(item.agent_id || '');
  const text = String(item.text || 'Model switched');
  deps.chatDispatch({
    type: 'ADD_MESSAGE',
    message: {
      role: 'agent' as const,
      from: 'system',
      to: '',
      text: `\u26A0\uFE0F ${text}`,
      timestamp: new Date().toLocaleTimeString(),
      timestampMs: Date.now(),
      isGenerating: false,
    },
  });
  if (agentId) {
    deps.setAgentStatusText((prev) => ({
      ...prev,
      [agentId]: `Fallback: ${item.data?.actual_model || 'alternate model'}`,
    }));
  }
}

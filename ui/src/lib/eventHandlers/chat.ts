import type { UiEvent, ContentBlock, SubagentToolStep } from '../../types';
import { useChatStore } from '../../stores/chatStore';
import { useServerStore } from '../../stores/serverStore';
import { useInteractionStore } from '../../stores/interactionStore';
import type { AgentStatusValue } from '../../stores/serverStore';
import { agentTracker } from '../agentTracker';
import {
  stripEmbeddedStructuredJson,
  isStatusLineText,
  shouldHideInternalChatMessage,
} from '../messageUtils';
import { getSessionId, formatToolStartLine } from './_shared';

// ---------------------------------------------------------------------------
// Text segment
// ---------------------------------------------------------------------------

export function handleTextSegment(item: UiEvent): void {
  const agentId = String(item.agent_id || '');
  if (!agentId) return;
  if (agentTracker.getParent(agentId)) return;
  const segText = String(item.text || '').trim();
  if (!segText) return;
  useChatStore.getState().addTextSegment(agentId, segText);
}

// ---------------------------------------------------------------------------
// Token — batched to rAF to avoid "Maximum update depth exceeded"
// ---------------------------------------------------------------------------

const _tokenBuffer: Map<string, { text: string; isThinking: boolean }> = new Map();
let _tokenFlushScheduled = false;

function flushTokenBuffer(): void {
  _tokenFlushScheduled = false;
  if (_tokenBuffer.size === 0) return;
  const chatStore = useChatStore.getState();
  for (const [agentId, { text, isThinking }] of _tokenBuffer) {
    if (text) chatStore.appendToken(agentId, text, isThinking);
  }
  _tokenBuffer.clear();
  useServerStore.getState().recomputeTokenRate();
}

export function handleToken(item: UiEvent): void {
  const agentId = String(item.agent_id || '');
  const isThinking = item.data?.thinking === true;
  if (!agentId) return;

  if (agentTracker.getParent(agentId)) return;

  if (item.phase === 'done') {
    // Flush any pending tokens before marking thinking done
    if (_tokenBuffer.size > 0) flushTokenBuffer();
    if (isThinking) {
      useChatStore.getState().setThinkingFlag(agentId);
    }
    return;
  }

  const tokenText = String(item.text || '');
  if (!isThinking && tokenText) {
    useServerStore.getState().recordTokenEvent();
  }

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

// ---------------------------------------------------------------------------
// Message (finalized chat message)
// ---------------------------------------------------------------------------

export function handleMessage(item: UiEvent): void {
  const agentId = String(item.agent_id || '');
  let content = String(item.text || '');
  if (!content) return;

  // Subagent terminal text: route into the parent's subagent tree entry
  // (revealed on click-to-expand) instead of leaking into the main chat.
  // Match the same tracker lookup the Token / TextSegment / ContentBlock
  // handlers use — by `item.agent_id` (or the explicit run_id when the
  // event carries one), NOT by `data.from` (which can be a role like
  // "assistant" and never matches the tracker). The previous fix here
  // checked `from` and silently failed for ENCODED status lines.
  if (agentId) {
    const subagentTrackingId = String(item.data?.run_id || agentId);
    const subagentParent =
      agentTracker.getParent(subagentTrackingId) || agentTracker.getParent(agentId);
    if (subagentParent) {
      useChatStore.getState().updateSubagentTree(
        subagentParent,
        subagentTrackingId,
        (entry) => ({ ...entry, resultText: content }),
      );
      return;
    }
  }

  const from = String(item.data?.from || item.agent_id || 'assistant');
  const to = String(item.data?.to || '');
  if (shouldHideInternalChatMessage(from, content)) return;

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
  const sid = getSessionId(item);
  const { elapsed: msgElapsed, contextTokens: msgCtxTokens } = sid ? agentTracker.clearRun(sid) : {};

  const isError = from !== 'user' && content.startsWith('Error:');
  chatStore.finalizeMessage(from, content, to, tsMs, msgElapsed, msgCtxTokens, isError || undefined);
}

// ---------------------------------------------------------------------------
// Content block (tool_use / text blocks, streamed with phase)
// ---------------------------------------------------------------------------

export function handleContentBlock(item: UiEvent): void {
  const agentId = String(item.agent_id || '');
  if (!agentId) return;
  const data = item.data || {};

  // Route subagent content blocks to parent tree. Prefer run_id as the
  // tracking key so parallel subagents with the same agent_id don't collide.
  const runIdFromData = data?.run_id ? String(data.run_id) : null;
  const trackingId = runIdFromData || agentId;
  const parentId = agentTracker.getParent(trackingId);
  if (parentId) {
    applySubagentContentBlock(parentId, trackingId, item.phase, data);
    return;
  }

  if (item.phase === 'start') {
    applyContentBlockStart(item, data);
    return;
  }
  if (item.phase === 'update') {
    applyContentBlockUpdate(agentId, data);
  }
}

function applySubagentContentBlock(
  parentId: string,
  agentId: string,
  phase: string | undefined,
  data: any,
): void {
  const chatStore = useChatStore.getState();

  if (phase === 'start' && data.block_type === 'tool_use') {
    const toolCount = agentTracker.incrementToolCount(agentId);
    const newStep: SubagentToolStep = {
      toolName: data.tool || 'Tool',
      args: data.args || '',
      status: 'running',
      timestampMs: Date.now(),
    };
    chatStore.updateSubagentTree(parentId, agentId,
      (entry) => ({
        ...entry,
        toolCount: toolCount || entry.toolCount + 1,
        currentActivity: `${data.tool || 'Tool'}: ${data.args || ''}`,
        toolSteps: [...(entry.toolSteps || []), newStep],
      }));
    return;
  }

  if (phase === 'update') {
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
}

function applyContentBlockStart(item: UiEvent, data: any): void {
  const agentId = String(item.agent_id || '');
  const chatStore = useChatStore.getState();
  const blockType = String(data.block_type || 'text');

  const block: ContentBlock = {
    type: blockType as ContentBlock['type'],
    timestampMs: Date.now(),
    id: String(data.block_id || ''),
    tool: data.tool || undefined,
    args: data.args || undefined,
    status: blockType === 'tool_use' ? 'running' : undefined,
    text: blockType === 'text' ? (data.args || '') : undefined,
  };
  chatStore.contentBlockStart(agentId, block);

  if (blockType !== 'tool_use') return;

  const toolName = data.tool || 'Tool';
  const toolArgs = data.args || '';
  const activityLine = formatToolStartLine(toolName, toolArgs);
  chatStore.appendActivity(agentId, activityLine);

  const sid = getSessionId(item);
  if (!sid) return;
  const agentStore = useServerStore.getState();
  agentStore.setAgentStatus((prev) => ({ ...prev, [sid]: 'calling_tool' as AgentStatusValue }));
  agentStore.setAgentStatusText((prev) => ({ ...prev, [sid]: activityLine }));
  agentTracker.ensureRunStarted(sid);
}

function applyContentBlockUpdate(agentId: string, data: any): void {
  const chatStore = useChatStore.getState();
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

// ---------------------------------------------------------------------------
// Turn complete
// ---------------------------------------------------------------------------

export function handleTurnComplete(item: UiEvent): void {
  const agentId = String(item.agent_id || '');
  if (!agentId) return;
  if (agentTracker.getParent(agentId)) return;

  const data = item.data || {};
  const durationMs = typeof data.duration_ms === 'number' ? data.duration_ms : undefined;
  const contextTokens = typeof data.context_tokens === 'number' ? data.context_tokens : undefined;

  const sid = getSessionId(item);
  const cleared = sid ? agentTracker.clearRun(sid) : {};
  const elapsed = durationMs || cleared.elapsed;
  const ctxTokens = contextTokens || cleared.contextTokens;

  useChatStore.getState().turnComplete(agentId, elapsed, ctxTokens);
  useInteractionStore.getState().setPendingAskUser(null);

  // Ensure status transitions to idle — the subsequent AgentStatus(idle)
  // event may arrive late or be missed, leaving the spinner stuck on "Thinking…".
  if (!sid) return;
  const agentStore = useServerStore.getState();
  agentStore.setAgentStatus((prev) => ({ ...prev, [sid]: 'idle' }));
  agentStore.setAgentStatusText((prev) => ({ ...prev, [sid]: 'Idle' }));
}

// ---------------------------------------------------------------------------
// Tool progress (stdout/stderr lines from running tools)
// ---------------------------------------------------------------------------

export function handleToolProgress(item: UiEvent): void {
  const agentId = String(item.agent_id || '');
  if (!agentId) return;
  if (agentTracker.getParent(agentId)) return;
  const line = String(item.data?.line || item.text || '');
  if (!line) return;
  useChatStore.getState().toolProgress(agentId, line);
}

import type { UiEvent, UiEventOf, ContentBlock, ContentBlockEvent, SubagentToolStep } from '../../types';
import type { ContentBlockStartData } from '../../types/generated/ContentBlockStartData';
import type { ContentBlockUpdateData } from '../../types/generated/ContentBlockUpdateData';
import { useChatStore } from '../../stores/chatStore';
import { useSessionStore } from '../../stores/sessionStore';
import { useServerStore } from '../../stores/serverStore';
import { useInteractionStore } from '../../stores/interactionStore';
import { useSuggestionStore } from '../../stores/suggestionStore';
import { agentTracker } from '../agentTracker';
import {
  stripEmbeddedStructuredJson,
  isStatusLineText,
  shouldHideInternalChatMessage,
} from '../messageUtils';
import { getSessionId, formatToolStartLine, runOf } from './_shared';
import { WORKING } from '../runStatus.mts';
import { askEndsWithTurn, completeAgentRuns, otherAgentRunning, sendEndsWithTurn } from '../agentTurns.mts';
import { stillOn } from '../sessionLoad.mts';
import { toolStatus } from '../toolStatus.mts';
import { pageImages } from '../chatImage.mts';

// ---------------------------------------------------------------------------
// Text segment
// ---------------------------------------------------------------------------

export function handleTextSegment(item: UiEventOf<'text_segment'>): void {
  const agentId = String(item.agent_id || '');
  if (!agentId) return;
  if (agentTracker.getParent(agentId)) return;
  const segText = String(item.text || '').trim();
  if (!segText) return;
  useChatStore.getState().addTextSegment(agentId, segText, runOf(item));
}

// ---------------------------------------------------------------------------
// Next-prompt hint — forked after the turn, so it can land late
// ---------------------------------------------------------------------------

export function handleFollowups(item: UiEventOf<'followups'>): void {
  const sid = item.session_id;
  if (!sid) return;
  if (agentTracker.getParent(String(item.agent_id || ''))) return;
  const runId = typeof item.data?.run_id === 'string' ? item.data.run_id : null;
  if (sessionMovedOn(sid, runId)) return;
  useSuggestionStore.getState().setHint(sid, item.data?.items ?? []);
}

/** A hint answers the turn it was forked from. Once they sent again, or a
 *  newer turn is running (a queued one starts on its own), it is stale. */
export function sessionMovedOn(sid: string, runId: string | null): boolean {
  const { pendingSends, agentRuns } = useServerStore.getState();
  if (pendingSends[sid]) return true;
  return agentRuns.some((r) => r.session_id === sid && r.status === 'running'
    && !r.parent_run_id && r.run_id !== runId);
}

// ---------------------------------------------------------------------------
// Token — batched to rAF to avoid "Maximum update depth exceeded"
// ---------------------------------------------------------------------------

/** Buffered per agent and run; each entry keeps the session its tokens came
 *  for, and a flush that lands after a switch drops them rather than stream
 *  them into the chat now on screen. */
type TokenBatch = { agentId: string; runId?: string; text: string; isThinking: boolean; sid: string | null };
const _tokenBuffer: Map<string, TokenBatch> = new Map();
let _tokenFlushScheduled = false;

function flushTokenBuffer(): void {
  _tokenFlushScheduled = false;
  if (_tokenBuffer.size === 0) return;
  const chatStore = useChatStore.getState();
  const open = useSessionStore.getState().activeSessionId;
  for (const { agentId, runId, text, isThinking, sid } of _tokenBuffer.values()) {
    if (text && stillOn(sid, open)) chatStore.appendToken(agentId, text, isThinking, runId);
  }
  _tokenBuffer.clear();
  useServerStore.getState().recomputeTokenRate();
}

export function handleToken(item: UiEventOf<'token'>): void {
  const agentId = String(item.agent_id || '');
  const isThinking = item.data?.thinking === true;
  if (!agentId) return;

  if (agentTracker.getParent(agentId)) return;

  if (item.phase === 'done') {
    // Flush any pending tokens before marking thinking done
    if (_tokenBuffer.size > 0) flushTokenBuffer();
    if (isThinking) {
      useChatStore.getState().setThinkingFlag(agentId, runOf(item));
    }
    return;
  }

  const tokenText = String(item.text || '');
  if (!isThinking && tokenText) {
    useServerStore.getState().recordTokenEvent();
  }

  const sid = useSessionStore.getState().activeSessionId;
  const runId = runOf(item);
  const key = `${agentId}|${runId || ''}`;
  const existing = _tokenBuffer.get(key);
  if (existing && existing.isThinking === isThinking && existing.sid === sid) {
    existing.text += tokenText;
  } else {
    // Flush if thinking state (or the session on screen) changed for this agent
    if (existing) flushTokenBuffer();
    _tokenBuffer.set(key, { agentId, runId, text: tokenText, isThinking, sid });
  }

  if (!_tokenFlushScheduled) {
    _tokenFlushScheduled = true;
    requestAnimationFrame(flushTokenBuffer);
  }
}

// ---------------------------------------------------------------------------
// Message (finalized chat message)
// ---------------------------------------------------------------------------

export function handleMessage(item: UiEventOf<'message'>): void {
  const agentId = String(item.agent_id || '');
  let content = String(item.text || '');
  if (!content) return;

  // Subagent terminal text: route into the parent's subagent tree entry
  // (revealed on click-to-expand) instead of leaking into the main chat.
  // Detection order: tracker hit by run_id or agent_id, then explicit
  // parent_id from the event payload as a race fallback (Message can
  // arrive before SubagentSpawned has registered the tracker).
  if (agentId) {
    const subagentTrackingId = String(item.data?.run_id || agentId);
    const parentFromData = item.data?.parent_agent_id ? String(item.data.parent_agent_id) : null;
    const subagentParent =
      agentTracker.getParent(subagentTrackingId) ||
      agentTracker.getParent(agentId) ||
      (parentFromData ? parentFromData.toLowerCase() : null);
    if (subagentParent) {
      useChatStore.getState().updateSubagentTree(
        subagentParent,
        subagentTrackingId,
        (entry) => ({ ...entry, resultText: content,
          ...(content.startsWith('Error:') ? { status:'failed' as const } : {}) }),
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
  // A message this surface sent, as the engine kept it: its own bubble takes
  // the kept words (matched by the id it was sent with), no second row.
  const clientId = from === 'user' ? item.data?.client_id : undefined;
  // A person's pictures, as the engine kept them — drawn as the turn lands.
  const images = from === 'user' ? pageImages(item.data?.images) : [];
  if (clientId && chatStore.confirmSent(clientId, content, to, images)) return;
  const runId = from !== 'user' ? runOf(item) : undefined;
  if (from !== 'user' && isStatusLineText(content)) {
    chatStore.appendActivity(from, content, runId);
    return;
  }

  const tsMs = Number(item.ts_ms || Date.now());
  const sid = getSessionId(item);
  const { elapsed: msgElapsed, contextTokens: msgCtxTokens } = sid ? agentTracker.clearRun(sid) : {};

  // The contract's silence: the turn ends with no words — its bubble keeps
  // its calls and is marked, and a turn with no bubble on screen adds none.
  if (from !== 'user' && item.data?.silent) {
    const onScreen = runId
      ? chatStore.messages.some((m) => m.live && m.runId === runId)
      : chatStore.messages.some((m) => m.from === from && m.isGenerating);
    if (!onScreen) return;
    chatStore.finalizeMessage(from, '', to, tsMs, msgElapsed, msgCtxTokens,
      undefined, undefined, undefined, undefined, undefined, runId);
    chatStore.markLastSilent(from, runId);
    return;
  }

  const isError = from !== 'user' && content.startsWith('Error:');
  chatStore.finalizeMessage(from, content, to, tsMs, msgElapsed, msgCtxTokens, isError || undefined,
    from === 'user' && item.data?.via ? String(item.data.via) : undefined,
    from === 'user' && item.data?.via_note ? String(item.data.via_note) : undefined,
    clientId ? String(clientId) : undefined,
    images.length ? images : undefined,
    runId);
}

// ---------------------------------------------------------------------------
// Content block (tool_use / text blocks, streamed with phase)
// ---------------------------------------------------------------------------

export function handleContentBlock(item: UiEventOf<'content_block'>): void {
  const agentId = String(item.agent_id || '');
  if (!agentId) return;
  const data = item.data;

  // Route subagent content blocks to parent tree. Prefer run_id as the
  // tracking key so parallel subagents with the same agent_id don't collide.
  const runIdFromData = data?.run_id ? String(data.run_id) : null;
  const trackingId = runIdFromData || agentId;
  const parentId = agentTracker.getParent(trackingId);
  if (parentId) {
    applySubagentContentBlock(parentId, trackingId, item);
    return;
  }
  if (!item.data) return;

  if (item.phase === 'start') {
    applyContentBlockStart(item, item.data);
    return;
  }
  if (item.phase === 'update') {
    applyContentBlockUpdate(agentId, item.data, runOf(item));
  }
}

/** The extra fields a tool's update carries flat beside the fixed ones
 *  (Edit/Write diffs, Bash output). */
interface ToolExtras {
  diff_type?: 'edit' | 'write';
  path?: string;
  old_string?: string;
  new_string?: string;
  new_content?: string;
  start_line?: number;
  lines_written?: number;
  bash_output?: string[];
}

function applySubagentContentBlock(
  parentId: string,
  agentId: string,
  item: ContentBlockEvent,
): void {
  const chatStore = useChatStore.getState();

  if (item.phase === 'start' && item.data?.block_type === 'tool_use') {
    const data = item.data;
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

  if (item.phase === 'update' && item.data) {
    const data = item.data;
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

function applyContentBlockStart(item: UiEvent, data: ContentBlockStartData): void {
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
  const runId = runOf(item);
  chatStore.contentBlockStart(agentId, block, runId);

  if (blockType !== 'tool_use') return;

  const toolName = data.tool || 'Tool';
  const toolArgs = data.args || '';
  const activityLine = formatToolStartLine(toolName, toolArgs);
  chatStore.appendActivity(agentId, activityLine, runId);

  const sid = getSessionId(item);
  if (!sid) return;
  const agentStore = useServerStore.getState();
  // The spinner says only that the agent works; the tool row says what.
  agentStore.setAgentStatusText((prev) => ({ ...prev, [sid]: WORKING }));
  agentTracker.ensureRunStarted(sid);
}

function applyContentBlockUpdate(agentId: string, update: ContentBlockUpdateData, runId?: string): void {
  const chatStore = useChatStore.getState();
  const data = update as ContentBlockUpdateData & ToolExtras;
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
    toolStatus(data.status),
    data.summary || undefined,
    data.is_error ?? undefined,
    diffData,
    Array.isArray(data.bash_output) ? data.bash_output as string[] : undefined,
    runId,
  );

  if (data.summary) {
    chatStore.appendActivity(agentId, data.summary, runId);
  }
}

// ---------------------------------------------------------------------------
// Turn complete
// ---------------------------------------------------------------------------

export function handleTurnComplete(item: UiEventOf<'turn_complete'>): void {
  const agentId = String(item.agent_id || '');
  if (!agentId) return;
  if (agentTracker.getParent(agentId)) return;

  const data = item.data;
  const durationMs = typeof data?.duration_ms === 'number' ? data.duration_ms : undefined;
  const contextTokens = typeof data?.context_tokens === 'number' ? data.context_tokens : undefined;

  // One app chat holds more than one agent's turns (another member
  // answering an `@name`): this turn's end is this agent's alone. Mark its runs done
  // first, then ask whether anyone else is still at work.
  const sid = getSessionId(item);
  if (sid) {
    const { agentRuns } = useServerStore.getState();
    useServerStore.setState({ agentRuns: completeAgentRuns(agentRuns, sid, agentId, Date.now()) });
  }
  const othersRunning = !!sid && otherAgentRunning(useServerStore.getState().agentRuns, sid, agentId);

  const cleared = sid && !othersRunning ? agentTracker.clearRun(sid) : {};
  const elapsed = durationMs || cleared.elapsed;
  const ctxTokens = contextTokens || cleared.contextTokens;

  useChatStore.getState().turnComplete(agentId, elapsed, ctxTokens, runOf(item));
  const interaction = useInteractionStore.getState();
  if (sid) {
    for (const ask of interaction.askQueues[sid] ?? []) {
      if (askEndsWithTurn(ask.agentId, agentId, (id) => agentTracker.getParent(id))) interaction.removeAsk(sid, ask.questionId);
    }
  }

  if (!sid) return;
  // SDK turns emit turn_complete without a native run outcome. Reload their
  // persisted estimate here as well, so it appears without a page refresh.
  if (agentId === 'claude' && stillOn(sid, useSessionStore.getState().activeSessionId)) {
    void useChatStore.getState().fetchSessionState({ sessionId: sid });
  }
  const agentStore = useServerStore.getState();
  // Ensure status transitions to idle — the subsequent AgentStatus(idle)
  // event may arrive late or be missed, leaving the spinner stuck on
  // "Thinking…". Not while another agent's turn runs here.
  if (!othersRunning) agentStore.setAgentStatusText((prev) => ({ ...prev, [sid]: 'Idle' }));
  // Clear the optimistic pendingSend flag — when the send was this agent's.
  if (sendEndsWithTurn(agentStore.pendingSendAgents[sid], agentId)) agentStore.setPendingSend(sid, false);
}

/**
 * Flip a subagent's `running` row in the local `agentRuns` mirror to
 * `completed` on its SubagentResult. The next page_state push will
 * replace this with the server-authoritative state.
 */
function markRunsCompletedForSession(
  sessionId: string,
  opts: { subagentRunId: string },
): void {
  const store = useServerStore.getState();
  const next = store.agentRuns.map((r) => {
    if (r.session_id !== sessionId) return r;
    if (r.status !== 'running') return r;
    if (r.run_id !== opts.subagentRunId) return r;
    return { ...r, status: 'completed' as const, ended_at: Date.now() };
  });
  useServerStore.setState({ agentRuns: next });
}
// Re-export for run.ts which needs the same helper for SubagentResult.
export { markRunsCompletedForSession };

// ---------------------------------------------------------------------------
// Tool progress (stdout/stderr lines from running tools)
// ---------------------------------------------------------------------------

export function handleToolProgress(item: UiEventOf<'tool_progress'>): void {
  const agentId = String(item.agent_id || '');
  if (!agentId) return;
  if (agentTracker.getParent(agentId)) return;
  const line = String(item.data?.line || item.text || '');
  if (!line) return;
  useChatStore.getState().toolProgress(agentId, line, runOf(item));
}

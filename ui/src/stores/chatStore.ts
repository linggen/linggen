/**
 * Chat messages state — absorbs the useChatMessages reducer.
 *
 * All message mutations go through named methods instead of a dispatch function.
 * The auto-scroll logic stays in React (needs DOM refs).
 */
import { create } from 'zustand';
import type { AgentRunInfo, ChatMessage, ContentBlock, SubagentTreeEntry, SessionState } from '../types';
import { rowForRun, settleTree, subagentHostIndex, unshownChildren } from '../lib/subagentRuns.mts';
import { isToolStatusText } from '../components/chat/MessagePhase';
import { dedupFetch } from '../lib/dedupFetch';
import {
  stripEmbeddedStructuredJson,
  isStatusLineText,
  isTransientStatus,
  summarizeActivityEntries,
  findLastGeneratingMessageIndex,
  upsertGeneratingAgentMessage,
  appendGeneratingActivity,
  updateParentSubagentTree,
  isPlanMessage,
  mergeChatMessages,
  shouldHideInternalChatMessage,
  isPersistedToolOnlyMessage,
} from '../lib/messageUtils';
import { cacheImages, restoreImages, clearImageCache } from '../lib/imageCache';
import { interruptedRunMessage, isInterruptedRunMeta, type RunMeta } from '../lib/interruptedRun.mts';
import { savedToolBlocks } from '../lib/toolStatus.mts';
import { agentTracker } from '../lib/agentTracker';
import { confirmSent } from '../lib/sentMessage.mts';
import { isEventEcho } from '../lib/rowIdentity.mts';
import { bubbleOfRun, claimForRun, isRunBubble, runBubbleIndex, sealedSegment } from '../lib/runBinding.mts';
import { loadOutcome, stillOn, type LoadSnapshot } from '../lib/sessionLoad.mts';
import { computeDisplay, mutate, mutateLast } from './chatMutationHelpers';
import { useSessionStore } from './sessionStore';
import { useUserStore } from './userStore';
import { useInteractionStore } from './interactionStore';

interface ChatState {
  messages: ChatMessage[];
  sessionState: SessionState | null;
  /** The session `sessionState` was loaded for, and what it held. */
  _sessionStateLoad: LoadSnapshot | null;
  /** The session whose load last answered (or failed) — until it is the one
   *  open, the chat is loading, not empty. */
  loadedSessionId: string | null;

  // Per-session message storage — avoids clear/refetch race on session switch
  _messagesBySession: Record<string, ChatMessage[]>;
  _activeSessionId: string | null;

  // Clear cooldown
  _chatClearTs: number;

  // Derived (recomputed on every mutation)
  displayMessages: ChatMessage[];

  // Session switching — swaps the active bucket without clear/refetch
  setActiveSession: (sessionId: string | null) => void;
  // Actions — one per reducer action type
  clear: (withCooldown?: boolean) => void;
  syncPersisted: (persisted: ChatMessage[]) => void;
  addMessage: (message: ChatMessage) => void;
  removeLastUserMessage: (text: string, agentId: string) => void;
  /** Drop one message — matched by timestamp and text, as rows are rebuilt on merge. */
  removeMessage: (target: ChatMessage) => void;
  // `runId`: the run the event came from (`run_id`) — it lands in that run's
  // bubble alone (lib/runBinding.mts); none for an event no run sent.
  upsertGenerating: (agentId: string, text: string, activityLine?: string, runId?: string) => void;
  appendActivity: (agentId: string, activityLine: string, runId?: string) => void;
  appendActivityWithSegments: (agentId: string, activityLine: string, runId?: string) => void;
  setPlaceholder: (agentId: string, text: string, runId?: string) => void;
  appendToken: (agentId: string, tokenText: string, isThinking: boolean, runId?: string) => void;
  setThinkingFlag: (agentId: string, runId?: string) => void;
  addTextSegment: (agentId: string, text: string, runId?: string) => void;
  upsertPlan: (agentId: string, planText: string) => void;
  updateSubagentTree: (parentId: string, subagentId: string, updater: (entry: SubagentTreeEntry) => SubagentTreeEntry) => void;
  /** A child's row, in its parent run's bubble (by run id, else by name). */
  addSubagentToTree: (parentId: string, entry: SubagentTreeEntry, parentRunId?: string | null) => void;
  /** The person's message joined `agentId`'s running turn: the work so far
   *  stays above it as its own block; the run goes on in a new bubble. */
  sealRunBubble: (agentId: string) => void;
  /** A native session's subagent rows against its runs (`subagentRuns.mts`). */
  reconcileSubagents: (runs: AgentRunInfo[]) => void;
  finalizeMessage: (agentId: string, content: string, to: string, tsMs: number, elapsed?: number, ctxTokens?: number, isError?: boolean, via?: string, viaNote?: string, clientId?: string, images?: string[], runId?: string) => void;
  /** The engine kept a message this surface sent (its bubble's `clientId`):
   *  the bubble takes the kept words. False when no bubble has that id. */
  confirmSent: (clientId: string, text: string, to: string, images?: string[]) => boolean;
  finalizeOnIdle: (agentId: string, elapsed?: number, ctxTokens?: number, runId?: string) => void;
  /** The agent's latest row was the spoken contract's silence. */
  markLastSilent: (agentId: string, runId?: string) => void;
  contentBlockStart: (agentId: string, block: ContentBlock, runId?: string) => void;
  contentBlockUpdate: (agentId: string, blockId: string, status?: ContentBlock['status'], summary?: string, isError?: boolean, diffData?: ContentBlock['diffData'], bashOutput?: string[], runId?: string) => void;
  toolProgress: (agentId: string, line: string, runId?: string) => void;
  turnComplete: (agentId: string, durationMs?: number, contextTokens?: number, runId?: string) => void;
  finalizeAllGenerating: (olderThanMs?: number) => void;

  // Workspace state
  fetchSessionState: (opts?: { projectRoot?: string; sessionId?: string }) => Promise<void>;

  // Helpers
  isInClearCooldown: () => boolean;
}

function loadSnapshot(sessionId: string, data: SessionState): LoadSnapshot {
  return {
    sessionId,
    messageCount: data?.messages?.length ?? 0,
    estimatedCost: data?.claude_usage?.estimated_cost_usd,
    agentStatus: data?.agent_status,
    planStatus: data?.plan_status,
  };
}

/** A finished turn's tool count: its tool calls, else (no blocks — an
 *  older event stream) its non-transient activity lines. Activity lines
 *  hold each call's start AND its result summary, so they are not a count
 *  when blocks exist. */
function toolCountOf(msg: ChatMessage): number {
  const blocks = (msg.content || []).filter((b) => b.type === 'tool_use').length;
  if (blocks > 0) return blocks;
  const entries = Array.isArray(msg.activityEntries) ? msg.activityEntries : [];
  return entries.filter((e: string) => !isTransientStatus(e)).length || msg.toolCount || 0;
}

export const useChatStore = create<ChatState>((set, get) => ({
  messages: [],
  sessionState: null,
  _sessionStateLoad: null,
  loadedSessionId: null,
  _messagesBySession: {},
  _activeSessionId: null,
  _chatClearTs: 0,
  displayMessages: [],

  setActiveSession: (sessionId) => {
    agentTracker.reset();
    const sid = sessionId || '__none__';
    const msgs = get()._messagesBySession[sid] || [];
    set({
      _activeSessionId: sessionId,
      messages: msgs,
      displayMessages: computeDisplay(msgs),
    });
  },

  clear: (withCooldown = true) => {
    const sid = get()._activeSessionId || '__none__';
    clearImageCache();
    agentTracker.reset();
    const bySession = { ...get()._messagesBySession };
    delete bySession[sid];
    set({ messages: [], displayMessages: [], sessionState: null, _sessionStateLoad: null, _messagesBySession: bySession, ...(withCooldown ? { _chatClearTs: Date.now() } : {}) });
  },

  syncPersisted: (persisted) => set(mutate((msgs) => {
    const merged = mergeChatMessages(persisted, msgs);
    return merged.map(restoreImages);
  })),

  addMessage: (message) => {
    cacheImages(message);
    set(mutate((msgs) => [...msgs, message]));
  },

  removeLastUserMessage: (text, _agentId) => set(mutate((msgs) => {
    // Match last user message by trimmed text (agentId match relaxed for robustness)
    const trimmed = text.trim();
    const idx = msgs.findLastIndex((m) => m.role === 'user' && m.text.trim() === trimmed);
    if (idx < 0) return msgs;
    const next = [...msgs];
    next.splice(idx, 1);
    return next;
  })),

  removeMessage: (target) => set(mutate((msgs) => {
    const idx = msgs.findIndex((m) => m.timestampMs === target.timestampMs && m.text === target.text);
    if (idx < 0) return msgs;
    const next = [...msgs];
    next.splice(idx, 1);
    return next;
  })),

  upsertGenerating: (agentId, text, activityLine, runId) =>
    set(mutate((msgs) => upsertGeneratingAgentMessage(msgs, agentId, text, activityLine, runId))),

  appendActivity: (agentId, activityLine, runId) =>
    set(mutate((msgs) => appendGeneratingActivity(msgs, agentId, activityLine, runId))),

  appendActivityWithSegments: (agentId, activityLine, runId) => set(mutate((msgs) => {
    const updated = appendGeneratingActivity(msgs, agentId, activityLine, runId);
    if (isTransientStatus(activityLine)) return updated;
    const idx = findLastGeneratingMessageIndex(updated, agentId, runId);
    if (idx < 0 || !updated[idx].segments) return updated;
    const next = [...updated];
    const msg = { ...next[idx] };
    const segs = [...(msg.segments || [])];
    const last = segs[segs.length - 1];
    if (last?.type === 'tools') {
      segs[segs.length - 1] = { ...last, entries: [...(last.entries || []), activityLine] };
    } else {
      segs.push({ type: 'tools', entries: [activityLine] });
    }
    msg.segments = segs;
    next[idx] = msg;
    return next;
  })),

  setPlaceholder: (agentId, text, runId) => set(mutateLast((state) => {
    const idx = findLastGeneratingMessageIndex(state, agentId, runId);
    if (idx >= 0 && state[idx].segments) return claimForRun(state, idx, runId);
    const updated = upsertGeneratingAgentMessage(state, agentId, text, undefined, runId);
    const uIdx = findLastGeneratingMessageIndex(updated, agentId, runId);
    if (uIdx >= 0) {
      const next = [...updated];
      next[uIdx] = { ...next[uIdx], isThinking: true };
      return next;
    }
    return updated;
  })),

  appendToken: (agentId, tokenText, isThinking, runId) => set(mutateLast((state) => {
    const idx = findLastGeneratingMessageIndex(state, agentId, runId);
    if (idx >= 0) {
      const next = claimForRun([...state], idx, runId);
      if (next[idx].segments) {
        if (isThinking) return state;
        next[idx] = {
          ...next[idx],
          liveText: (next[idx].liveText || '') + tokenText,
          isGenerating: true,
          isThinking: false,
        };
        return next;
      }
      if (isThinking) {
        if (!next[idx].isThinking) {
          next[idx] = { ...next[idx], isThinking: true, isGenerating: true };
        }
        return next;
      }
      const wasThinking = next[idx].isThinking;
      const isPlaceholder = isStatusLineText(next[idx].text || '');
      const shouldReplace = wasThinking || isPlaceholder;
      next[idx] = {
        ...next[idx],
        text: shouldReplace ? tokenText : (next[idx].text || '') + tokenText,
        isGenerating: true,
        isThinking: false,
      };
      return next;
    }
    if (isThinking) {
      const created = upsertGeneratingAgentMessage(state, agentId, 'Thinking...', undefined, runId);
      const cIdx = findLastGeneratingMessageIndex(created, agentId, runId);
      if (cIdx >= 0) {
        const next = [...created];
        next[cIdx] = { ...next[cIdx], isThinking: true };
        return next;
      }
      return created;
    }
    return upsertGeneratingAgentMessage(state, agentId, tokenText, undefined, runId);
  })),

  setThinkingFlag: (agentId, runId) => set(mutate((state) => {
    const idx = findLastGeneratingMessageIndex(state, agentId, runId);
    if (idx >= 0) {
      const next = claimForRun([...state], idx, runId);
      if (next[idx].segments) return next;
      next[idx] = { ...next[idx], isThinking: true };
      return next;
    }
    return state;
  })),

  addTextSegment: (agentId, text, runId) => set(mutate((state) => {
    const idx = findLastGeneratingMessageIndex(state, agentId, runId);
    if (idx < 0) return upsertGeneratingAgentMessage(state, agentId, '', undefined, runId);
    const next = claimForRun([...state], idx, runId);
    const msg = { ...next[idx] };
    msg.segments = [...(msg.segments || []), { type: 'text' as const, text }];
    msg.liveText = '';
    next[idx] = msg;
    return next;
  })),

  upsertPlan: (agentId, planText) => set(mutate((state) => {
    // Find existing plan message to update in-place (e.g. during execution
    // progress updates, or when ExitPlanMode refines a plan after feedback).
    const existingIdx = state.findIndex((m) => {
      if ((m.from || m.role) !== agentId) return false;
      return isPlanMessage(m);
    });
    if (existingIdx >= 0) {
      const next = [...state];
      next[existingIdx] = {
        ...next[existingIdx],
        text: planText,
        timestampMs: Date.now(),
        timestamp: new Date().toLocaleTimeString(),
      };
      return next;
    }
    // First PlanUpdate (from ExitPlanMode). Insert as new message.
    // No streaming to morph — plan text streaming is disabled.
    return [...state, {
      role: 'agent' as const,
      from: agentId,
      to: 'user',
      text: planText,
      timestamp: new Date().toLocaleTimeString(),
      timestampMs: Date.now(),
    }];
  })),

  updateSubagentTree: (parentId, subagentId, updater) =>
    set(mutate((msgs) => updateParentSubagentTree(msgs, parentId, subagentId, updater))),

  addSubagentToTree: (parentId, entry, parentRunId) => set(mutate((state) => {
    const targetIdx = subagentHostIndex(state, parentId, parentRunId);
    if (targetIdx < 0) return state;
    const next = [...state];
    const msg = next[targetIdx];
    const tree = msg.subagentTree ? [...msg.subagentTree] : [];
    const existing = tree.findIndex((item) => item.subagentId === entry.subagentId);
    if (existing < 0) tree.push(entry);
    else tree[existing] = { ...tree[existing], agentName:entry.agentName, task:entry.task || tree[existing].task };
    next[targetIdx] = { ...msg, subagentTree: tree };
    return next;
  })),

  sealRunBubble: (agentId) => set(mutate((msgs) => {
    const idx = msgs.findIndex((m) => isRunBubble(m) && m.isGenerating && m.from === agentId);
    if (idx < 0) return msgs;
    const next = [...msgs];
    next[idx] = sealedSegment(next[idx]);
    return next;
  })),

  reconcileSubagents: (runs) => {
    const live = new Set(runs.filter((run) => run.status === 'running').map((run) => run.run_id));
    const now = Date.now();
    const shown = new Set<string>();
    set(mutate((msgs) => {
      let changed = false;
      const next = msgs.map((msg) => {
        if (!msg.subagentTree?.length) return msg;
        for (const entry of msg.subagentTree) shown.add(entry.subagentId);
        const tree = settleTree(msg.subagentTree, live, now);
        if (tree === msg.subagentTree) return msg;
        changed = true;
        return { ...msg, subagentTree: tree };
      });
      return changed ? next : msgs;
    }));
    for (const child of unshownChildren(runs, shown)) {
      const parent = runs.find((run) => run.run_id === child.parent_run_id);
      get().addSubagentToTree((parent?.agent_id ?? child.agent_id).toLowerCase(), rowForRun(child), child.parent_run_id);
    }
  },

  finalizeMessage: (agentId, content, to, tsMs, elapsed, ctxTokens, isError, via, viaNote, clientId, images, runId) => set(mutate((state) => {
    const generatingIdx = findLastGeneratingMessageIndex(state, agentId, runId);
    if (generatingIdx >= 0) {
      const next = claimForRun([...state], generatingIdx, runId);
      const existingMsg = next[generatingIdx];
      const existingEntries = Array.isArray(existingMsg.activityEntries) ? existingMsg.activityEntries : [];
      const nonTransient = existingEntries.filter((e: string) => !isTransientStatus(e));

      const hasRunningSubagents = (existingMsg.subagentTree || []).some((e) => e.status === 'running');
      const hasRunningBlocks = (existingMsg.content || []).some((b) => b.type === 'tool_use' && b.status === 'running');
      const keepGenerating = hasRunningSubagents || hasRunningBlocks;

      let finalSegments = existingMsg.segments;
      // Only append a text segment if the message is still generating.
      // If turn_complete already finalized it, the segments are complete —
      // appending the server's final text would duplicate visible content.
      if (finalSegments && existingMsg.isGenerating && content.trim()) {
        const strippedContent = content
          .split('\n')
          .filter((line: string) => {
            const t = line.trimStart();
            return !t.startsWith('Used tool:') && !t.startsWith('Delegated task:') && !isToolStatusText(t);
          })
          .join('\n')
          .replace(/\n{3,}/g, '\n\n')
          .trim();
        if (strippedContent) {
          const lastTextSeg = [...finalSegments].reverse().find((s) => s.type === 'text');
          if (!lastTextSeg || lastTextSeg.text !== strippedContent) {
            finalSegments = [...finalSegments, { type: 'text' as const, text: strippedContent }];
          }
        }
      }
      const keepTs = existingMsg.timestampMs && existingMsg.timestampMs > 0;
      // Don't overwrite plan message text — upsertPlan already set it
      // and finalizeMessage runs after, which would destroy the plan JSON.
      const keepText = isPlanMessage(existingMsg);
      next[generatingIdx] = {
        ...existingMsg,
        text: keepText ? existingMsg.text : content,
        to: to || existingMsg.to || 'user',
        isGenerating: keepGenerating,
        isThinking: false,
        isError: isError || existingMsg.isError,
        liveText: keepGenerating ? existingMsg.liveText : undefined,
        segments: finalSegments,
        timestamp: keepTs ? existingMsg.timestamp : new Date(tsMs).toLocaleTimeString(),
        timestampMs: keepTs ? existingMsg.timestampMs : tsMs,
        activitySummary: summarizeActivityEntries(existingEntries, keepGenerating) || existingMsg.activitySummary,
        toolCount: nonTransient.length || existingMsg.toolCount,
        durationMs: elapsed || existingMsg.durationMs,
        contextTokens: ctxTokens || existingMsg.contextTokens,
      };
      return next;
    }

    const row: ChatMessage = {
      role: agentId === 'user' ? 'user' : 'agent',
      from: agentId,
      to,
      text: content,
      timestamp: new Date(tsMs).toLocaleTimeString(),
      timestampMs: tsMs,
      isGenerating: false,
      isError,
      ...(clientId ? { clientId } : {}),
      ...(via ? { via } : {}),
      ...(viaNote ? { viaNote } : {}),
      ...(images?.length ? { images } : {}),
      ...bubbleOfRun(state, runId),
    };
    // A run's words with no bubble here: the run's row, bound by its id
    // (runBinding.mts). Else a row already here (by its sent id, or the same
    // row again) is not added twice; the same words said again are a new row.
    if (!runId && isEventEcho(state, row)) return state;
    return [...state, row];
  })),

  confirmSent: (clientId, text, to, images) => {
    let found = false;
    set(mutate((msgs) => {
      const next = confirmSent(msgs, clientId, text, to, images);
      found = next !== null;
      return next ?? msgs;
    }));
    return found;
  },

  markLastSilent: (agentId, runId) => set(mutate((state) => {
    const idx = runId ? runBubbleIndex(state, agentId, runId) : state.map((m) => m.from).lastIndexOf(agentId);
    if (idx < 0) return state;
    const next = [...state];
    next[idx] = { ...next[idx], silent: true, text: '', liveText: undefined };
    return next;
  })),

  finalizeOnIdle: (agentId, elapsed, ctxTokens, runId) => set(mutate((state) => {
    const idx = findLastGeneratingMessageIndex(state, agentId, runId);
    if (idx < 0 || !state[idx].isGenerating) return state;
    const tree = state[idx].subagentTree;
    if (tree && tree.some((e) => e.status === 'running')) return state;
    const next = [...state];
    const msg = next[idx];
    const entries = Array.isArray(msg.activityEntries) ? msg.activityEntries : [];
    const finalText = msg.text || msg.liveText || '';
    next[idx] = {
      ...msg,
      text: finalText,
      isGenerating: false,
      isThinking: false,
      liveText: undefined,
      activitySummary: summarizeActivityEntries(entries, false) || msg.activitySummary,
      toolCount: toolCountOf(msg),
      durationMs: elapsed || msg.durationMs,
      contextTokens: ctxTokens || msg.contextTokens,
    };
    return next;
  })),

  contentBlockStart: (agentId, block, runId) => set(mutate((state) => {
    const idx = findLastGeneratingMessageIndex(state, agentId, runId);
    if (idx >= 0) {
      const next = claimForRun([...state], idx, runId);
      const msg = { ...next[idx] };
      const blocks = [...(msg.content || [])];
      blocks.push(block);
      msg.content = blocks;
      msg.isGenerating = true;
      msg.isThinking = false;
      if (block.type === 'tool_use') {
        const segs = [...(msg.segments || [])];
        const statusLine = block.args ? `${block.tool || 'Tool'}: ${block.args}` : `${block.tool || 'Tool'}`;
        const last = segs[segs.length - 1];
        if (last?.type === 'tools') {
          segs[segs.length - 1] = { ...last, entries: [...(last.entries || []), statusLine] };
        } else {
          segs.push({ type: 'tools', entries: [statusLine] });
        }
        msg.segments = segs;
      } else if (block.type === 'text' && block.text) {
        const segs = [...(msg.segments || [])];
        segs.push({ type: 'text' as const, text: block.text });
        msg.segments = segs;
        msg.liveText = '';
      }
      next[idx] = msg;
      return next;
    }
    const now = new Date();
    const newMsg: ChatMessage = {
      role: 'agent',
      from: agentId,
      to: 'user',
      text: '',
      timestamp: now.toLocaleTimeString(),
      timestampMs: now.getTime(),
      isGenerating: true,
      content: [block],
      segments: block.type === 'tool_use'
        ? [{ type: 'tools', entries: [block.args ? `${block.tool || 'Tool'}: ${block.args}` : `${block.tool || 'Tool'}`] }]
        : block.type === 'text' && block.text
          ? [{ type: 'text', text: block.text }]
          : [],
      ...bubbleOfRun(state, runId),
    };
    return [...state, newMsg];
  })),

  contentBlockUpdate: (agentId, blockId, status, summary, isError, diffData, bashOutput, runId) => set(mutate((state) => {
    const idx = findLastGeneratingMessageIndex(state, agentId, runId);
    if (idx < 0) return state;
    const next = [...state];
    const msg = { ...next[idx] };
    const blocks = [...(msg.content || [])];
    const blockIdx = blocks.findIndex((b) => b.id === blockId);
    if (blockIdx >= 0) {
      const existingOutput = blocks[blockIdx].output;
      const mergedOutput = (!existingOutput || existingOutput.length === 0) && bashOutput ? bashOutput : existingOutput;
      blocks[blockIdx] = {
        ...blocks[blockIdx],
        status: status || blocks[blockIdx].status,
        summary: summary || blocks[blockIdx].summary,
        isError: isError ?? blocks[blockIdx].isError,
        diffData: diffData || blocks[blockIdx].diffData,
        output: mergedOutput,
      };
      msg.content = blocks;
    }
    if (summary && msg.segments) {
      const segs = [...msg.segments];
      for (let si = segs.length - 1; si >= 0; si--) {
        if (segs[si].type === 'tools' && segs[si].entries) {
          const entries = [...(segs[si].entries || [])];
          for (let ei = entries.length - 1; ei >= 0; ei--) {
            const block = blocks.find((b) => b.id === blockId);
            if (block?.tool && entries[ei].startsWith(block.tool)) {
              entries[ei] = summary;
              break;
            }
          }
          segs[si] = { ...segs[si], entries };
          break;
        }
      }
      msg.segments = segs;
    }
    next[idx] = msg;
    return next;
  })),

  toolProgress: (agentId, line, runId) => set(mutateLast((state) => {
    let idx = findLastGeneratingMessageIndex(state, agentId, runId);
    if (idx < 0 && !runId) {
      for (let i = state.length - 1; i >= 0; i--) {
        if (state[i].from === agentId) { idx = i; break; }
      }
    }
    if (idx < 0) return state;
    const next = [...state];
    const msg = { ...next[idx] };
    const blocks = [...(msg.content || [])];
    let targetIdx = -1;
    for (let i = blocks.length - 1; i >= 0; i--) {
      if (blocks[i].type === 'tool_use' && blocks[i].status === 'running') {
        targetIdx = i;
        break;
      }
    }
    if (targetIdx < 0) {
      for (let i = blocks.length - 1; i >= 0; i--) {
        if (blocks[i].type === 'tool_use' && blocks[i].tool === 'Bash') {
          targetIdx = i;
          break;
        }
      }
    }
    if (targetIdx >= 0) {
      const existingOutput = blocks[targetIdx].output || [];
      const newOutput = existingOutput.length >= 500
        ? [...existingOutput.slice(1), line]
        : [...existingOutput, line];
      blocks[targetIdx] = { ...blocks[targetIdx], output: newOutput };
      msg.content = blocks;
      next[idx] = msg;
      return next;
    }
    return state;
  })),

  // Reconnect heal: a transport reconnect is a hard discontinuity — an
  // in-flight turn either died with the daemon (restart mid-turn) or its
  // TurnComplete was lost, so ghost "Thinking…" bubbles would spin forever.
  // Finalize every generating message: keep ones with real content, drop
  // bare placeholders (nothing streamed — nothing happened). If the run
  // actually survived (network blip), the next token recreates the bubble
  // and the final text is restored from persisted state at turn end.
  finalizeAllGenerating: (olderThanMs = 0) => set(mutate((state) => {
    if (!state.some((m) => m.isGenerating)) return state;
    const cutoff = olderThanMs > 0 ? Date.now() - olderThanMs : Infinity;
    return state.flatMap((m) => {
      if (!m.isGenerating) return [m];
      // Age gate (page_state reconcile path): a bubble updated recently may
      // belong to a run the stale page_state snapshot doesn't know yet —
      // leave it; only frozen bubbles get finalized.
      if ((m.timestampMs ?? 0) > cutoff) return [m];
      const text = m.liveText || (isStatusLineText(m.text || '') ? '' : m.text) || '';
      const hasContent = !!text || (m.content || []).length > 0 || !!m.segments;
      if (!hasContent) return [];
      // wasFrozenStream keeps the bubble claimable by its persisted row in
      // mergeChatMessages even though it's no longer isGenerating.
      return [{ ...m, text, isGenerating: false, isThinking: false, liveText: undefined, wasFrozenStream: true }];
    });
  })),

  turnComplete: (agentId, durationMs, contextTokens, runId) => set(mutate((state) => {
    const idx = findLastGeneratingMessageIndex(state, agentId, runId);
    if (idx < 0) return state;
    const next = [...state];
    const msg = next[idx];
    // Already ended by its run's outcome: the turn's end adds only its stats.
    if (!msg.isGenerating) {
      if (!durationMs && !contextTokens) return state;
      next[idx] = { ...msg, durationMs: msg.durationMs || durationMs, contextTokens: msg.contextTokens || contextTokens };
      return next;
    }

    const hasRunningSubagents = (msg.subagentTree || []).some((e) => e.status === 'running');
    if (hasRunningSubagents) {
      next[idx] = {
        ...msg,
        durationMs: durationMs || msg.durationMs,
        contextTokens: contextTokens || msg.contextTokens,
      };
      return next;
    }

    const entries = Array.isArray(msg.activityEntries) ? msg.activityEntries : [];
    const totalTools = toolCountOf(msg);
    const textIsPlaceholder = isStatusLineText(msg.text || '');
    // Recover final text from multiple sources:
    // 1. liveText (token-accumulated streaming text)
    // 2. msg.text (if not a placeholder like "Thinking...")
    // 3. msg.segments text entries (set by addTextSegment from text_segment events)
    const segmentsText = (msg.segments || [])
      .filter((s: { type: string; text?: string }) => s.type === 'text' && s.text)
      .map((s: { text?: string }) => s.text)
      .join('\n\n');
    const finalText = msg.liveText || (textIsPlaceholder ? '' : msg.text) || segmentsText || '';

    next[idx] = {
      ...msg,
      text: finalText,
      isGenerating: false,
      isThinking: false,
      liveText: undefined,
      activitySummary: summarizeActivityEntries(entries, false) || msg.activitySummary,
      toolCount: totalTools,
      durationMs: durationMs || msg.durationMs,
      contextTokens: contextTokens || msg.contextTokens,
    };
    return next;
  })),

  fetchSessionState: async (opts) => {
    // Wait until we know who the user is — 'pending' means user_info hasn't arrived yet.
    const perm = useUserStore.getState().userPermission;
    if (perm === 'pending') return;

    const projectState = useSessionStore.getState();
    let selectedProjectRoot = opts?.projectRoot ?? projectState.selectedProjectRoot;
    const activeSessionId = opts?.sessionId ?? projectState.activeSessionId;
    const { isMissionSession, activeMissionId, isSkillSession, activeSkillName } = projectState;
    if (!activeSessionId) return;
    if (isMissionSession && !activeMissionId) return;
    if (isSkillSession && !activeSkillName) return;
    // When no project is selected, fall back to the active session's cwd/project.
    if (!isMissionSession && !isSkillSession && !selectedProjectRoot) {
      const sess = projectState.allSessions.find((s) => s.id === activeSessionId);
      selectedProjectRoot = sess?.project || sess?.cwd || '';
    }
    try {
      let url: URL;
      if (isMissionSession && activeMissionId) {
        url = new URL('/api/missions/sessions/state', window.location.origin);
        url.searchParams.append('mission_id', activeMissionId);
        url.searchParams.append('session_id', activeSessionId);
      } else if (isSkillSession && activeSkillName) {
        url = new URL('/api/skill-sessions/state', window.location.origin);
        url.searchParams.append('skill', activeSkillName);
        url.searchParams.append('session_id', activeSessionId);
      } else {
        url = new URL('/api/workspace/state', window.location.origin);
        url.searchParams.append('project_root', selectedProjectRoot);
        url.searchParams.append('session_id', activeSessionId);
      }

      const resp = await dedupFetch(url.toString());
      const data: SessionState = await resp.json();
      // The load answers the session it asked for: dropped when the person
      // moved on meanwhile, skipped when nothing changed (re-render loops).
      const loaded = loadSnapshot(activeSessionId, data);
      const outcome = loadOutcome(loaded, useSessionStore.getState().activeSessionId, get()._sessionStateLoad);
      if (outcome === 'stale') return;
      set({ loadedSessionId: activeSessionId });
      if (outcome !== 'apply') return;
      set({ sessionState: data, _sessionStateLoad: loaded });

      const state = get();
      if (data.messages && !state.isInClearCooldown()) {
        const msgs: ChatMessage[] = data.messages
          .filter(([meta, body]) => !shouldHideInternalChatMessage(meta.from, body))
          .filter(([_meta, body]) => !isPersistedToolOnlyMessage(String(body || '')))
          .flatMap(([meta, body], idx) => {
            // A run that stopped before it replied: its calls, marked interrupted.
            if (isInterruptedRunMeta(meta as RunMeta)) return [interruptedRunMessage(meta as RunMeta)];
            // The run that wrote the row: it binds to that run's bubble by it.
            const runId = typeof meta.run_id === 'string' ? { runId: meta.run_id } : {};
            const isUser = meta.from === 'user' || meta.from === 'system';
            let bodyStr = String(body || '');

            try {
              const parsed = JSON.parse(bodyStr);
              if (parsed?.type === 'plan' && parsed?.plan) {
                // Normalize the from field (strip run- prefix) so it matches
                // the live plan message created by upsertPlan during streaming.
                const rawFrom = String(meta.from || '');
                const fromMatch = rawFrom.match(/^run-(.+?)-\d+/);
                const normalizedFrom = fromMatch ? fromMatch[1] : rawFrom;
                return [{
                  role: 'agent' as const,
                  from: normalizedFrom,
                  to: meta.to,
                  text: String(body || ''),
                  timestamp: new Date(meta.ts * 1000).toLocaleTimeString(),
                  timestampMs: Number(meta.ts || 0) * 1000,
                }];
              }
            } catch { /* not pure JSON */ }

            if (!isUser) {
              bodyStr = stripEmbeddedStructuredJson(bodyStr);
            }
            // The calls made before these words, with the status each ended in.
            // The live run's row (a page loaded mid-run) keeps its calls running.
            const live = (meta as RunMeta).run === 'running';
            const tools = !isUser ? savedToolBlocks((meta as RunMeta).tools, `${meta.ts}-${idx}`, live) : [];
            if (!isUser && !bodyStr && !tools.length) return [];
            const isError = !isUser && bodyStr.startsWith('Error:');
            return [{
              role: meta.from === 'user' ? 'user' : 'agent',
              from: meta.from,
              to: meta.to,
              text: bodyStr,
              timestamp: new Date(meta.ts * 1000).toLocaleTimeString(),
              timestampMs: Number(meta.ts || 0) * 1000,
              ...runId,
              ...(tools.length ? { content: tools, toolCount: tools.length } : {}),
              ...(Array.isArray(meta.subagents) ? { subagentTree: meta.subagents as SubagentTreeEntry[] } : {}),
              ...(isError ? { isError: true } : {}),
              ...(meta.client_id ? { clientId: String(meta.client_id) } : {}),
              ...(meta.via ? { via: String(meta.via) } : {}),
              ...(meta.via_note ? { viaNote: String(meta.via_note) } : {}),
              // The pictures the row kept: the engine's URLs for each.
              ...(Array.isArray(meta.images) && meta.images.length ? { images: meta.images.map(String) } : {}),
            }];
          });
        state.syncPersisted(msgs);

        // Restore pending plan state from persisted messages (e.g. after server restart).
        // Find the MOST RECENT plan message — only set as pending if its status is "planned".
        // If the most recent plan is approved/executing/completed/rejected, there's no pending plan.
        const interactionState = useInteractionStore.getState();
        if (!interactionState.pendingPlanAgentId) {
          for (let i = msgs.length - 1; i >= 0; i--) {
            const m = msgs[i];
            if (!isPlanMessage(m)) continue;
            try {
              const parsed = JSON.parse(m.text);
              if (parsed?.plan?.status === 'planned') {
                interactionState.setPendingPlan(parsed.plan);
                interactionState.setPendingPlanAgentId(m.from || m.role || '');
              }
              // Stop at the first plan found regardless of status
              break;
            } catch { /* ignore */ }
          }
        }
      }
    } catch (e) {
      console.error('Error fetching workspace state:', e);
      if (stillOn(activeSessionId, useSessionStore.getState().activeSessionId)) set({ loadedSessionId: activeSessionId });
    }
  },

  isInClearCooldown: () => {
    const ts = get()._chatClearTs;
    return ts > 0 && Date.now() - ts < 5000;
  },
}));

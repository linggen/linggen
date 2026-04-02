/**
 * Chat messages state — absorbs the useChatMessages reducer.
 *
 * All message mutations go through named methods instead of a dispatch function.
 * The auto-scroll logic stays in React (needs DOM refs).
 */
import { create } from 'zustand';
import type { ChatMessage, ContentBlock, SubagentTreeEntry, WorkspaceState } from '../types';
import { isToolStatusText } from '../components/chat/MessagePhase';
import { dedupFetch } from '../lib/dedupFetch';
import {
  stripEmbeddedStructuredJson,
  isStatusLineText,
  isTransientStatus,
  normalizeMessageTextForDedup,
  summarizeActivityEntries,
  findLastGeneratingMessageIndex,
  upsertGeneratingAgentMessage,
  appendGeneratingActivity,
  updateParentSubagentTree,
  isPlanMessage,
  dedupPlanMessages,
  mergeChatMessages,
  shouldHideInternalChatMessage,
  isPersistedToolOnlyMessage,
  reconstructContentFromText,
} from '../lib/messageUtils';
import { useProjectStore } from './projectStore';
import { useUiStore } from './uiStore';

interface ChatState {
  messages: ChatMessage[];
  workspaceState: WorkspaceState | null;

  // Per-session message storage — avoids clear/refetch race on session switch
  _messagesBySession: Record<string, ChatMessage[]>;
  _activeSessionId: string | null;

  // Clear cooldown
  _chatClearTs: number;

  // Derived (recomputed on every mutation)
  displayMessages: ChatMessage[];

  // Session switching — swaps the active bucket without clear/refetch
  setActiveSession: (sessionId: string | null) => void;
  /** Check whether messages are already cached for a session. */
  hasSessionMessages: (sessionId: string | null) => boolean;

  // Actions — one per reducer action type
  clear: (withCooldown?: boolean) => void;
  syncPersisted: (persisted: ChatMessage[]) => void;
  addMessage: (message: ChatMessage) => void;
  removeLastUserMessage: (text: string, agentId: string) => void;
  upsertGenerating: (agentId: string, text: string, activityLine?: string) => void;
  appendActivity: (agentId: string, activityLine: string) => void;
  appendActivityWithSegments: (agentId: string, activityLine: string) => void;
  setPlaceholder: (agentId: string, text: string) => void;
  appendToken: (agentId: string, tokenText: string, isThinking: boolean) => void;
  setThinkingFlag: (agentId: string) => void;
  addTextSegment: (agentId: string, text: string) => void;
  upsertPlan: (agentId: string, planText: string) => void;
  updateSubagentTree: (parentId: string, subagentId: string, updater: (entry: SubagentTreeEntry) => SubagentTreeEntry) => void;
  addSubagentToTree: (parentId: string, entry: SubagentTreeEntry) => void;
  finalizeMessage: (agentId: string, content: string, to: string, tsMs: number, elapsed?: number, ctxTokens?: number, isError?: boolean) => void;
  finalizeOnIdle: (agentId: string, elapsed?: number, ctxTokens?: number) => void;
  contentBlockStart: (agentId: string, block: ContentBlock) => void;
  contentBlockUpdate: (agentId: string, blockId: string, status?: ContentBlock['status'], summary?: string, isError?: boolean, diffData?: ContentBlock['diffData'], bashOutput?: string[]) => void;
  toolProgress: (agentId: string, line: string) => void;
  turnComplete: (agentId: string, durationMs?: number, contextTokens?: number) => void;

  // Workspace state
  fetchWorkspaceState: (opts?: { projectRoot?: string; sessionId?: string }) => Promise<void>;

  // Helpers
  isInClearCooldown: () => boolean;
}

/** Compute display messages from raw messages. */
function computeDisplay(messages: ChatMessage[]): ChatMessage[] {
  // Hide [BOARD_MOVE] messages — internal game state not meant for display
  const visible = messages.filter(m => !(m.role === 'user' && m.text?.startsWith('[BOARD_MOVE]')));
  const deduped = dedupPlanMessages(visible);
  const plans: ChatMessage[] = [];
  const nonPlans: ChatMessage[] = [];
  for (const msg of deduped) {
    if (isPlanMessage(msg)) {
      plans.push(msg);
    } else {
      if ((msg.from || msg.role) !== 'user' && !(msg.content && msg.content.length > 0)) {
        let text = msg.text || '';
        text = stripEmbeddedStructuredJson(text);
        if (msg.isGenerating) {
          text = text.replace(/\{\s*"type\b[^}]*$/s, '').trim();
        }
        if (text !== msg.text && text) {
          nonPlans.push({ ...msg, text });
          continue;
        }
      }
      nonPlans.push(msg);
    }
  }
  return [...nonPlans, ...plans];
}

/** Apply a mutation to the active session's messages and recompute displayMessages. */
function mutate(fn: (msgs: ChatMessage[]) => ChatMessage[]): (s: ChatState) => Partial<ChatState> {
  return (s) => {
    const sid = s._activeSessionId || '__none__';
    const currentMsgs = s._messagesBySession[sid] || [];
    const messages = fn(currentMsgs);
    const displayMessages = computeDisplay(messages);
    return {
      messages,
      displayMessages,
      _messagesBySession: { ...s._messagesBySession, [sid]: messages },
    };
  };
}

/**
 * Fast-path mutation for token streaming: updates only the last message in
 * both `messages` and `displayMessages` without running `computeDisplay` over
 * the full array.  Falls back to `mutate` when the last message changes identity
 * (new message added, message removed, etc.).
 */
function mutateLast(fn: (msgs: ChatMessage[]) => ChatMessage[]): (s: ChatState) => Partial<ChatState> {
  return (s) => {
    const sid = s._activeSessionId || '__none__';
    const currentMsgs = s._messagesBySession[sid] || [];
    const messages = fn(currentMsgs);

    // Fast path: if the array length is the same and only the last element changed,
    // patch displayMessages in-place instead of recomputing from scratch.
    if (
      messages.length === currentMsgs.length &&
      messages.length > 0 &&
      s.displayMessages.length > 0
    ) {
      const lastNew = messages[messages.length - 1];
      const lastDisplay = s.displayMessages[s.displayMessages.length - 1];
      // Match by identity: same role+from means we're updating the same bubble.
      if (lastDisplay && lastNew.role === lastDisplay.role && lastNew.from === lastDisplay.from) {
        const displayMessages = [...s.displayMessages];
        displayMessages[displayMessages.length - 1] = lastNew;
        return {
          messages,
          displayMessages,
          _messagesBySession: { ...s._messagesBySession, [sid]: messages },
        };
      }
    }

    // Fallback: full recompute (new message was added, etc.)
    const displayMessages = computeDisplay(messages);
    return {
      messages,
      displayMessages,
      _messagesBySession: { ...s._messagesBySession, [sid]: messages },
    };
  };
}

// Client-side image cache: images are ephemeral (never persisted to disk),
// so we cache them here keyed by "from::normalizedText" to survive merges
// and state reloads. Cleared on /clear.
const _imageCache = new Map<string, string[]>();

function imageCacheKey(from: string, text: string): string {
  return `${from}::${normalizeMessageTextForDedup(text)}`;
}

/** Store images from a message into the cache. */
function cacheImages(msg: ChatMessage): void {
  if (msg.images && msg.images.length > 0) {
    _imageCache.set(imageCacheKey(msg.from || msg.role || '', msg.text), msg.images);
  }
}

/** Restore images from the cache onto a message if it's missing them. */
function restoreImages(msg: ChatMessage): ChatMessage {
  if (msg.images && msg.images.length > 0) return msg;
  const cached = _imageCache.get(imageCacheKey(msg.from || msg.role || '', msg.text));
  return cached ? { ...msg, images: cached } : msg;
}

export const useChatStore = create<ChatState>((set, get) => ({
  messages: [],
  workspaceState: null,
  _messagesBySession: {},
  _activeSessionId: null,
  _chatClearTs: 0,
  displayMessages: [],

  setActiveSession: (sessionId) => {
    const sid = sessionId || '__none__';
    const msgs = get()._messagesBySession[sid] || [];
    set({
      _activeSessionId: sessionId,
      messages: msgs,
      displayMessages: computeDisplay(msgs),
    });
  },

  hasSessionMessages: (sessionId) => {
    const sid = sessionId || '__none__';
    const msgs = get()._messagesBySession[sid];
    return !!msgs && msgs.length > 0;
  },

  clear: (withCooldown = true) => {
    const sid = get()._activeSessionId || '__none__';
    _imageCache.clear();
    const bySession = { ...get()._messagesBySession };
    delete bySession[sid];
    set({ messages: [], displayMessages: [], _messagesBySession: bySession, ...(withCooldown ? { _chatClearTs: Date.now() } : {}) });
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

  upsertGenerating: (agentId, text, activityLine) =>
    set(mutate((msgs) => upsertGeneratingAgentMessage(msgs, agentId, text, activityLine))),

  appendActivity: (agentId, activityLine) =>
    set(mutate((msgs) => appendGeneratingActivity(msgs, agentId, activityLine))),

  appendActivityWithSegments: (agentId, activityLine) => set(mutate((msgs) => {
    const updated = appendGeneratingActivity(msgs, agentId, activityLine);
    if (isTransientStatus(activityLine)) return updated;
    const idx = findLastGeneratingMessageIndex(updated, agentId);
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

  setPlaceholder: (agentId, text) => set(mutateLast((state) => {
    const idx = findLastGeneratingMessageIndex(state, agentId);
    if (idx >= 0 && state[idx].segments) return state;
    const updated = upsertGeneratingAgentMessage(state, agentId, text);
    const uIdx = findLastGeneratingMessageIndex(updated, agentId);
    if (uIdx >= 0) {
      const next = [...updated];
      next[uIdx] = { ...next[uIdx], isThinking: true };
      return next;
    }
    return updated;
  })),

  appendToken: (agentId, tokenText, isThinking) => set(mutateLast((state) => {
    const idx = findLastGeneratingMessageIndex(state, agentId);
    if (idx >= 0) {
      const next = [...state];
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
      const created = upsertGeneratingAgentMessage(state, agentId, 'Thinking...');
      const cIdx = findLastGeneratingMessageIndex(created, agentId);
      if (cIdx >= 0) {
        const next = [...created];
        next[cIdx] = { ...next[cIdx], isThinking: true };
        return next;
      }
      return created;
    }
    return upsertGeneratingAgentMessage(state, agentId, tokenText);
  })),

  setThinkingFlag: (agentId) => set(mutate((state) => {
    const idx = findLastGeneratingMessageIndex(state, agentId);
    if (idx >= 0) {
      const next = [...state];
      if (next[idx].segments) return next;
      next[idx] = { ...next[idx], isThinking: true };
      return next;
    }
    return state;
  })),

  addTextSegment: (agentId, text) => set(mutate((state) => {
    const idx = findLastGeneratingMessageIndex(state, agentId);
    if (idx < 0) return upsertGeneratingAgentMessage(state, agentId, '');
    const next = [...state];
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
    // First PlanUpdate (from ExitPlanMode during planning). Morph the
    // generating message into the plan message in-place so the DOM
    // position stays the same — no flash.
    const genIdx = findLastGeneratingMessageIndex(state, agentId);
    if (genIdx >= 0) {
      const next = [...state];
      next[genIdx] = {
        ...next[genIdx],
        text: planText,
        liveText: undefined,
        isGenerating: undefined,
        timestampMs: Date.now(),
        timestamp: new Date().toLocaleTimeString(),
      };
      return next;
    }
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

  addSubagentToTree: (parentId, entry) => set(mutate((state) => {
    let targetIdx = -1;
    for (let i = state.length - 1; i >= 0; i--) {
      const m = state[i];
      if ((m.from || '').toLowerCase() === parentId && m.isGenerating) {
        targetIdx = i;
        break;
      }
    }
    if (targetIdx < 0) {
      for (let i = state.length - 1; i >= 0; i--) {
        const m = state[i];
        if ((m.from || '').toLowerCase() === parentId && m.role === 'agent') {
          targetIdx = i;
          break;
        }
      }
    }
    if (targetIdx < 0) return state;
    const next = [...state];
    const msg = next[targetIdx];
    const tree = msg.subagentTree ? [...msg.subagentTree] : [];
    tree.push(entry);
    next[targetIdx] = { ...msg, subagentTree: tree };
    return next;
  })),

  finalizeMessage: (agentId, content, to, tsMs, elapsed, ctxTokens, isError) => set(mutate((state) => {
    const generatingIdx = findLastGeneratingMessageIndex(state, agentId);
    if (generatingIdx >= 0) {
      const next = [...state];
      const existingMsg = next[generatingIdx];
      const existingEntries = Array.isArray(existingMsg.activityEntries) ? existingMsg.activityEntries : [];
      const nonTransient = existingEntries.filter((e: string) => !isTransientStatus(e));

      const hasRunningSubagents = (existingMsg.subagentTree || []).some((e) => e.status === 'running');
      const hasRunningBlocks = (existingMsg.content || []).some((b) => b.type === 'tool_use' && b.status === 'running');
      const keepGenerating = hasRunningSubagents || hasRunningBlocks;

      let finalSegments = existingMsg.segments;
      if (finalSegments && content.trim()) {
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
      next[generatingIdx] = {
        ...existingMsg,
        text: content,
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

    // Dedup check
    if (state.some((msg) =>
      !msg.isGenerating &&
      (msg.from || msg.role) === agentId &&
      (msg.to || '') === to &&
      normalizeMessageTextForDedup(msg.text) === normalizeMessageTextForDedup(content)
    )) return state;

    return [...state, {
      role: agentId === 'user' ? 'user' : 'agent',
      from: agentId,
      to,
      text: content,
      timestamp: new Date(tsMs).toLocaleTimeString(),
      timestampMs: tsMs,
      isGenerating: false,
      isError,
    }];
  })),

  finalizeOnIdle: (agentId, elapsed, ctxTokens) => set(mutate((state) => {
    const idx = findLastGeneratingMessageIndex(state, agentId);
    if (idx < 0) return state;
    const tree = state[idx].subagentTree;
    if (tree && tree.some((e) => e.status === 'running')) return state;
    const next = [...state];
    const msg = next[idx];
    const entries = Array.isArray(msg.activityEntries) ? msg.activityEntries : [];
    const nonTransient = entries.filter((e: string) => !isTransientStatus(e));
    const finalText = msg.text || msg.liveText || '';
    next[idx] = {
      ...msg,
      text: finalText,
      isGenerating: false,
      isThinking: false,
      liveText: undefined,
      activitySummary: summarizeActivityEntries(entries, false) || msg.activitySummary,
      toolCount: nonTransient.length || msg.toolCount,
      durationMs: elapsed || msg.durationMs,
      contextTokens: ctxTokens || msg.contextTokens,
    };
    return next;
  })),

  contentBlockStart: (agentId, block) => set(mutate((state) => {
    const idx = findLastGeneratingMessageIndex(state, agentId);
    if (idx >= 0) {
      const next = [...state];
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
    };
    return [...state, newMsg];
  })),

  contentBlockUpdate: (agentId, blockId, status, summary, isError, diffData, bashOutput) => set(mutate((state) => {
    const idx = findLastGeneratingMessageIndex(state, agentId);
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

  toolProgress: (agentId, line) => set(mutateLast((state) => {
    let idx = findLastGeneratingMessageIndex(state, agentId);
    if (idx < 0) {
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

  turnComplete: (agentId, durationMs, contextTokens) => set(mutate((state) => {
    const idx = findLastGeneratingMessageIndex(state, agentId);
    if (idx < 0) return state;
    const next = [...state];
    const msg = next[idx];

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
    const nonTransient = entries.filter((e: string) => !isTransientStatus(e));
    const toolBlocks = (msg.content || []).filter((b) => b.type === 'tool_use');
    const totalTools = toolBlocks.length || nonTransient.length || msg.toolCount || 0;
    const textIsPlaceholder = isStatusLineText(msg.text || '');
    // Recover final text from multiple sources:
    // 1. liveText (token-accumulated streaming text)
    // 2. msg.text (if not a placeholder like "Thinking...")
    // 3. msg.segments text entries (set by addTextSegment from text_segment SSE events)
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

  fetchWorkspaceState: async (opts) => {
    const projectState = useProjectStore.getState();
    let selectedProjectRoot = opts?.projectRoot ?? projectState.selectedProjectRoot;
    const activeSessionId = opts?.sessionId ?? projectState.activeSessionId;
    const { isMissionSession, activeMissionId, isSkillSession, activeSkillName } = projectState;
    if (!activeSessionId) return;
    if (isMissionSession && !activeMissionId) return;
    if (isSkillSession && !activeSkillName) return;
    // When no project is selected, fall back to the active session's cwd/project
    // so messages still load after page refresh in project-less workspaces.
    if (!isMissionSession && !isSkillSession && !selectedProjectRoot) {
      const sess = projectState.allSessions.find((s) => s.id === activeSessionId);
      selectedProjectRoot = sess?.project || sess?.cwd || '';
      if (!selectedProjectRoot) return;
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
      const data = await resp.json();
      // Skip update if workspace state hasn't meaningfully changed (prevents re-render loops)
      const prev = get().workspaceState;
      const prevMsgCount = prev?.messages?.length ?? 0;
      const newMsgCount = data?.messages?.length ?? 0;
      const prevStatus = prev?.agent_status;
      const newStatus = data?.agent_status;
      if (prevMsgCount === newMsgCount && prevStatus === newStatus && prev?.plan_status === data?.plan_status) return;
      set({ workspaceState: data });

      const state = get();
      if (data.messages && !state.isInClearCooldown()) {
        const msgs: ChatMessage[] = data.messages
          .filter(([meta, body]: any) => !shouldHideInternalChatMessage(meta.from, body))
          .filter(([_meta, body]: any) => !isPersistedToolOnlyMessage(String(body || '')))
          .flatMap(([meta, body]: any) => {
            const isUser = meta.from === 'user' || meta.from === 'system';
            let bodyStr = String(body || '');

            try {
              const parsed = JSON.parse(bodyStr);
              if (parsed?.type === 'plan' && parsed?.plan) {
                return [{
                  role: 'agent' as const,
                  from: meta.from,
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
            if (!isUser && !bodyStr) return [];
            const restored = !isUser ? reconstructContentFromText(bodyStr) : null;
            const isError = !isUser && bodyStr.startsWith('Error:');
            return [{
              role: meta.from === 'user' ? 'user' : 'agent',
              from: meta.from,
              to: meta.to,
              text: bodyStr,
              timestamp: new Date(meta.ts * 1000).toLocaleTimeString(),
              timestampMs: Number(meta.ts || 0) * 1000,
              ...(restored ? { content: restored.content, toolCount: restored.toolCount } : {}),
              ...(isError ? { isError: true } : {}),
            }];
          });
        state.syncPersisted(msgs);

        // Restore pending plan state from persisted messages (e.g. after server restart).
        // Find the MOST RECENT plan message — only set as pending if its status is "planned".
        // If the most recent plan is approved/executing/completed/rejected, there's no pending plan.
        const uiState = useUiStore.getState();
        if (!uiState.pendingPlanAgentId) {
          for (let i = msgs.length - 1; i >= 0; i--) {
            const m = msgs[i];
            if (!isPlanMessage(m)) continue;
            try {
              const parsed = JSON.parse(m.text);
              if (parsed?.plan?.status === 'planned') {
                uiState.setPendingPlan(parsed.plan);
                uiState.setPendingPlanAgentId(m.from || m.role || '');
              }
              // Stop at the first plan found regardless of status
              break;
            } catch { /* ignore */ }
          }
        }
      }
    } catch (e) {
      console.error('Error fetching workspace state:', e);
    }
  },

  isInClearCooldown: () => {
    const ts = get()._chatClearTs;
    return ts > 0 && Date.now() - ts < 5000;
  },
}));

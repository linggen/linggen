/**
 * Chat message state management via useReducer.
 * Single source of truth for all chat message mutations.
 */
import { useReducer, useMemo, useEffect, useRef } from 'react';
import type { ChatMessage, ContentBlock, SubagentTreeEntry } from '../types';
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
} from '../lib/messageUtils';

// ---------------------------------------------------------------------------
// Action types
// ---------------------------------------------------------------------------

export type ChatAction =
  | { type: 'CLEAR' }
  | { type: 'SYNC_PERSISTED'; persisted: ChatMessage[] }
  | { type: 'ADD_MESSAGE'; message: ChatMessage }
  | { type: 'REMOVE_LAST_USER_MESSAGE'; text: string; agentId: string }
  | { type: 'UPSERT_GENERATING'; agentId: string; text: string; activityLine?: string }
  | { type: 'APPEND_ACTIVITY'; agentId: string; activityLine: string }
  | { type: 'APPEND_ACTIVITY_WITH_SEGMENTS'; agentId: string; activityLine: string }
  | { type: 'SET_PLACEHOLDER'; agentId: string; text: string }
  | { type: 'APPEND_TOKEN'; agentId: string; tokenText: string; isThinking: boolean }
  | { type: 'SET_THINKING_FLAG'; agentId: string }
  | { type: 'ADD_TEXT_SEGMENT'; agentId: string; text: string }
  | { type: 'UPSERT_PLAN'; agentId: string; planText: string }
  | { type: 'UPDATE_SUBAGENT_TREE'; parentId: string; subagentId: string; updater: (entry: SubagentTreeEntry) => SubagentTreeEntry }
  | { type: 'ADD_SUBAGENT_TO_TREE'; parentId: string; entry: SubagentTreeEntry }
  | { type: 'FINALIZE_MESSAGE'; agentId: string; content: string; to: string; tsMs: number; elapsed?: number; ctxTokens?: number }
  | { type: 'FINALIZE_ON_IDLE'; agentId: string; elapsed?: number; ctxTokens?: number }
  | { type: 'CONTENT_BLOCK_START'; agentId: string; block: ContentBlock }
  | { type: 'CONTENT_BLOCK_UPDATE'; agentId: string; blockId: string; status?: string; summary?: string; isError?: boolean; diffData?: ContentBlock['diffData']; bashOutput?: string[] }
  | { type: 'TOOL_PROGRESS'; agentId: string; line: string }
  | { type: 'TURN_COMPLETE'; agentId: string; durationMs?: number; contextTokens?: number };

// ---------------------------------------------------------------------------
// Reducer
// ---------------------------------------------------------------------------

function chatReducer(state: ChatMessage[], action: ChatAction): ChatMessage[] {
  switch (action.type) {
    case 'CLEAR':
      return [];

    case 'SYNC_PERSISTED':
      return mergeChatMessages(action.persisted, state);

    case 'ADD_MESSAGE':
      return [...state, action.message];

    case 'REMOVE_LAST_USER_MESSAGE': {
      const idx = state.findLastIndex(
        (m) => m.role === 'user' && m.text === action.text && m.to === action.agentId
      );
      if (idx < 0) return state;
      const next = [...state];
      next.splice(idx, 1);
      return next;
    }

    case 'UPSERT_GENERATING':
      return upsertGeneratingAgentMessage(state, action.agentId, action.text, action.activityLine);

    case 'APPEND_ACTIVITY':
      return appendGeneratingActivity(state, action.agentId, action.activityLine);

    case 'APPEND_ACTIVITY_WITH_SEGMENTS': {
      const updated = appendGeneratingActivity(state, action.agentId, action.activityLine);
      // Don't add transient statuses (Thinking, Model loading) to segments
      if (isTransientStatus(action.activityLine)) return updated;
      const idx = findLastGeneratingMessageIndex(updated, action.agentId);
      if (idx < 0 || !updated[idx].segments) return updated;
      const next = [...updated];
      const msg = { ...next[idx] };
      const segs = [...(msg.segments || [])];
      const last = segs[segs.length - 1];
      if (last?.type === 'tools') {
        segs[segs.length - 1] = { ...last, entries: [...(last.entries || []), action.activityLine] };
      } else {
        segs.push({ type: 'tools', entries: [action.activityLine] });
      }
      msg.segments = segs;
      next[idx] = msg;
      return next;
    }

    case 'SET_PLACEHOLDER': {
      const idx = findLastGeneratingMessageIndex(state, action.agentId);
      if (idx >= 0 && state[idx].segments) return state;
      const updated = upsertGeneratingAgentMessage(state, action.agentId, action.text);
      // Mark as thinking so the UI can render ThinkingIndicator
      const uIdx = findLastGeneratingMessageIndex(updated, action.agentId);
      if (uIdx >= 0) {
        const next = [...updated];
        next[uIdx] = { ...next[uIdx], isThinking: true };
        return next;
      }
      return updated;
    }

    case 'APPEND_TOKEN': {
      const { agentId, tokenText, isThinking } = action;
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
        // Don't accumulate thinking tokens in msg.text — ThinkingIndicator
        // reads the placeholder text (e.g. "Thinking (model)") already set
        // by SET_PLACEHOLDER. This prevents raw thinking content from leaking
        // into the visible message body.
        if (isThinking) {
          if (!next[idx].isThinking) {
            next[idx] = { ...next[idx], isThinking: true, isGenerating: true };
          }
          return next;
        }
        // Non-thinking token: if previously thinking or placeholder, start fresh
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
      // No existing message — for thinking tokens, create a placeholder
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
    }

    case 'SET_THINKING_FLAG': {
      const idx = findLastGeneratingMessageIndex(state, action.agentId);
      if (idx >= 0) {
        const next = [...state];
        if (next[idx].segments) return next;
        next[idx] = { ...next[idx], isThinking: true };
        return next;
      }
      return state;
    }

    case 'ADD_TEXT_SEGMENT': {
      const idx = findLastGeneratingMessageIndex(state, action.agentId);
      if (idx < 0) return upsertGeneratingAgentMessage(state, action.agentId, '');
      const next = [...state];
      const msg = { ...next[idx] };
      msg.segments = [...(msg.segments || []), { type: 'text' as const, text: action.text }];
      msg.liveText = '';
      next[idx] = msg;
      return next;
    }

    case 'UPSERT_PLAN': {
      const existingIdx = state.findIndex((m) => {
        if ((m.from || m.role) !== action.agentId) return false;
        return isPlanMessage(m);
      });
      if (existingIdx >= 0) {
        const next = [...state];
        next[existingIdx] = {
          ...next[existingIdx],
          text: action.planText,
          timestampMs: Date.now(),
          timestamp: new Date().toLocaleTimeString(),
        };
        return next;
      }
      return [...state, {
        role: 'agent' as const,
        from: action.agentId,
        to: 'user',
        text: action.planText,
        timestamp: new Date().toLocaleTimeString(),
        timestampMs: Date.now(),
      }];
    }

    case 'UPDATE_SUBAGENT_TREE':
      return updateParentSubagentTree(state, action.parentId, action.subagentId, action.updater);

    case 'ADD_SUBAGENT_TO_TREE': {
      let targetIdx = -1;
      for (let i = state.length - 1; i >= 0; i--) {
        const m = state[i];
        if ((m.from || '').toLowerCase() === action.parentId && m.isGenerating) {
          targetIdx = i;
          break;
        }
      }
      if (targetIdx < 0) {
        for (let i = state.length - 1; i >= 0; i--) {
          const m = state[i];
          if ((m.from || '').toLowerCase() === action.parentId && m.role === 'agent') {
            targetIdx = i;
            break;
          }
        }
      }
      if (targetIdx < 0) return state;
      const next = [...state];
      const msg = next[targetIdx];
      const tree = msg.subagentTree ? [...msg.subagentTree] : [];
      tree.push(action.entry);
      next[targetIdx] = { ...msg, subagentTree: tree };
      return next;
    }

    case 'FINALIZE_MESSAGE': {
      const { agentId, content, to, tsMs, elapsed, ctxTokens } = action;
      const generatingIdx = findLastGeneratingMessageIndex(state, agentId);
      if (generatingIdx >= 0) {
        const next = [...state];
        const existingMsg = next[generatingIdx];
        const existingEntries = Array.isArray(existingMsg.activityEntries)
          ? existingMsg.activityEntries
          : [];
        const nonTransient = existingEntries.filter((e: string) => !isTransientStatus(e));

        // Don't finalize if subagents are still running — keep isGenerating true
        // so tokens from the parent agent can still be appended after subagent returns.
        const hasRunningSubagents = (existingMsg.subagentTree || []).some(
          (e) => e.status === 'running'
        );
        const hasRunningBlocks = (existingMsg.content || []).some(
          (b) => b.type === 'tool_use' && b.status === 'running'
        );
        const keepGenerating = hasRunningSubagents || hasRunningBlocks;

        let finalSegments = existingMsg.segments;
        if (finalSegments && content.trim()) {
          // Strip "Used tool:" and "Delegated task:" lines from the finalized content — tool info
          // is already tracked in tool segments, so including it again would
          // create duplicate entries in the segmented view.
          const strippedContent = content
            .split('\n')
            .filter((line: string) => {
              const t = line.trimStart();
              return !t.startsWith('Used tool:') && !t.startsWith('Delegated task:');
            })
            .join('\n')
            .replace(/\n{3,}/g, '\n\n')
            .trim();
          if (strippedContent) {
            const lastTextSeg = [...finalSegments].reverse().find(s => s.type === 'text');
            if (!lastTextSeg || lastTextSeg.text !== strippedContent) {
              finalSegments = [...finalSegments, { type: 'text' as const, text: strippedContent }];
            }
          }
        }
        next[generatingIdx] = {
          ...existingMsg,
          text: content,
          to: to || existingMsg.to || 'user',
          isGenerating: keepGenerating,
          isThinking: false,
          liveText: keepGenerating ? existingMsg.liveText : undefined,
          segments: finalSegments,
          timestamp: new Date(tsMs).toLocaleTimeString(),
          timestampMs: tsMs,
          activitySummary:
            summarizeActivityEntries(existingEntries, keepGenerating) || existingMsg.activitySummary,
          toolCount: nonTransient.length || existingMsg.toolCount,
          durationMs: elapsed || existingMsg.durationMs,
          contextTokens: ctxTokens || existingMsg.contextTokens,
        };
        return next;
      }

      // Dedup: if an identical finalized message already exists, skip
      if (
        state.some(
          (msg) =>
            !msg.isGenerating &&
            (msg.from || msg.role) === agentId &&
            (msg.to || '') === to &&
            normalizeMessageTextForDedup(msg.text) ===
              normalizeMessageTextForDedup(content)
        )
      ) {
        return state;
      }

      return [
        ...state,
        {
          role: agentId === 'user' ? 'user' : 'agent',
          from: agentId,
          to,
          text: content,
          timestamp: new Date(tsMs).toLocaleTimeString(),
          timestampMs: tsMs,
          isGenerating: false,
        },
      ];
    }

    case 'FINALIZE_ON_IDLE': {
      const { agentId, elapsed, ctxTokens } = action;
      const idx = findLastGeneratingMessageIndex(state, agentId);
      if (idx < 0) return state;
      const tree = state[idx].subagentTree;
      if (tree && tree.some((e) => e.status === 'running')) return state;
      const next = [...state];
      const entries = Array.isArray(next[idx].activityEntries) ? next[idx].activityEntries : [];
      const nonTransient = entries.filter((e: string) => !isTransientStatus(e));
      next[idx] = {
        ...next[idx],
        isGenerating: false,
        isThinking: false,
        liveText: undefined,
        activitySummary: summarizeActivityEntries(entries, false) || next[idx].activitySummary,
        toolCount: nonTransient.length || next[idx].toolCount,
        durationMs: elapsed || next[idx].durationMs,
        contextTokens: ctxTokens || next[idx].contextTokens,
      };
      return next;
    }

    case 'CONTENT_BLOCK_START': {
      const { agentId, block } = action;
      const idx = findLastGeneratingMessageIndex(state, agentId);
      if (idx >= 0) {
        const next = [...state];
        const msg = { ...next[idx] };
        const blocks = [...(msg.content || [])];
        blocks.push(block);
        msg.content = blocks;
        msg.isGenerating = true;
        msg.isThinking = false; // Clear thinking when any content block arrives
        // Also add to segments for backward compat rendering
        if (block.type === 'tool_use') {
          const segs = [...(msg.segments || [])];
          const statusLine = block.args
            ? `${block.tool || 'Tool'}: ${block.args}`
            : `${block.tool || 'Tool'}`;
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
      // No generating message yet — create one
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
    }

    case 'TOOL_PROGRESS': {
      const { agentId, line } = action;
      // Try generating message first, then fall back to last message from this agent
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
      // Find the last running tool_use block (the one currently executing)
      let targetIdx = -1;
      for (let i = blocks.length - 1; i >= 0; i--) {
        if (blocks[i].type === 'tool_use' && blocks[i].status === 'running') {
          targetIdx = i;
          break;
        }
      }
      // Fallback: if no running block, find the last Bash block (progress events
      // can arrive after ContentBlockUpdate marks the block as 'done')
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
        // Cap at 500 lines to prevent memory issues
        const newOutput = existingOutput.length >= 500
          ? [...existingOutput.slice(1), line]
          : [...existingOutput, line];
        blocks[targetIdx] = { ...blocks[targetIdx], output: newOutput };
        msg.content = blocks;
        next[idx] = msg;
        return next;
      }
      return state;
    }

    case 'CONTENT_BLOCK_UPDATE': {
      const { agentId, blockId, status, summary, isError, diffData, bashOutput } = action;
      const idx = findLastGeneratingMessageIndex(state, agentId);
      if (idx < 0) return state;
      const next = [...state];
      const msg = { ...next[idx] };
      const blocks = [...(msg.content || [])];
      const blockIdx = blocks.findIndex((b) => b.id === blockId);
      if (blockIdx >= 0) {
        // Merge bash output from the backend if no progress events populated it yet.
        const existingOutput = blocks[blockIdx].output;
        const mergedOutput = (!existingOutput || existingOutput.length === 0) && bashOutput
          ? bashOutput
          : existingOutput;
        blocks[blockIdx] = {
          ...blocks[blockIdx],
          status: (status as ContentBlock['status']) || blocks[blockIdx].status,
          summary: summary || blocks[blockIdx].summary,
          isError: isError ?? blocks[blockIdx].isError,
          diffData: diffData || blocks[blockIdx].diffData,
          output: mergedOutput,
        };
        msg.content = blocks;
      }
      // Update the corresponding segment entry with done status text
      if (summary && msg.segments) {
        const segs = [...msg.segments];
        // Find the last tools segment and update the last entry that matches this block
        for (let si = segs.length - 1; si >= 0; si--) {
          if (segs[si].type === 'tools' && segs[si].entries) {
            const entries = [...(segs[si].entries || [])];
            // Replace the last entry for this tool with the done summary
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
    }

    case 'TURN_COMPLETE': {
      const { agentId, durationMs, contextTokens } = action;
      const idx = findLastGeneratingMessageIndex(state, agentId);
      if (idx < 0) return state;
      const next = [...state];
      const msg = next[idx];

      // Don't finalize if subagents are still running — the parent will
      // continue generating after the subagent returns its result.
      const hasRunningSubagents = (msg.subagentTree || []).some(
        (e) => e.status === 'running'
      );
      if (hasRunningSubagents) {
        // Just update stats, keep generating
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
      next[idx] = {
        ...msg,
        isGenerating: false,
        isThinking: false,
        liveText: undefined,
        activitySummary: summarizeActivityEntries(entries, false) || msg.activitySummary,
        toolCount: totalTools,
        durationMs: durationMs || msg.durationMs,
        contextTokens: contextTokens || msg.contextTokens,
      };
      return next;
    }

    default:
      return state;
  }
}

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

export function useChatMessages() {
  const [chatMessages, chatDispatch] = useReducer(chatReducer, []);
  const chatEndRef = useRef<HTMLDivElement>(null);
  const lastChatCountRef = useRef(0);
  const chatClearTsRef = useRef(0);

  // Auto-scroll when new messages arrive
  useEffect(() => {
    if (chatMessages.length > lastChatCountRef.current) {
      chatEndRef.current?.scrollIntoView({ behavior: 'auto', block: 'nearest', inline: 'nearest' });
    }
    lastChatCountRef.current = chatMessages.length;
  }, [chatMessages.length]);

  // Track clear timestamp for cooldown
  const clearChat = () => {
    chatClearTsRef.current = Date.now();
    chatDispatch({ type: 'CLEAR' });
  };

  /** Check if we're in clear-cooldown (suppress persisted sync for 5s after clear). */
  const isInClearCooldown = () =>
    chatClearTsRef.current > 0 && Date.now() - chatClearTsRef.current < 5000;

  /** Always-deduped + structured-JSON-stripped messages for rendering.
   *  Plan messages are moved to the bottom and kept as one per agent.
   *  Messages with structured content blocks skip text sanitization. */
  const displayMessages = useMemo(() => {
    const deduped = dedupPlanMessages(chatMessages);
    const plans: ChatMessage[] = [];
    const nonPlans: ChatMessage[] = [];
    for (const msg of deduped) {
      if (isPlanMessage(msg)) {
        plans.push(msg);
      } else {
        // Messages with structured content blocks don't need text sanitization —
        // tool info is in ContentBlock[], not embedded as JSON in text.
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
  }, [chatMessages]);

  return {
    chatMessages,
    displayMessages,
    chatDispatch,
    chatEndRef,
    clearChat,
    isInClearCooldown,
  };
}

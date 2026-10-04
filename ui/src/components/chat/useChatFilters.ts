// Which messages the panel shows: the session's thread — every member's
// conversation (split into a stable finished part and the streaming row) —
// its queue, and the open subagent drawer's messages.
import { useMemo } from 'react';
import type { ChatMessage, QueuedChatItem, SubagentInfo } from '../../types';
import { useSessionStore } from '../../stores/sessionStore';
import { useStableArray } from '../../hooks/useStableArray';
import { normalizeAgentKey, sortMessagesByTime, collapseProgressMessages } from '../../lib/messageUtils';

/** A main-chat row: every member's — the session's thread is one table
 *  (doc/shared-session-spec.md) — so any main agent's traffic, the user's
 *  lines to any of them, and unrouted notices (system, compaction, skill-page
 *  `assistant` rows). `selected` is the agent the chat speaks with; its rows
 *  count even when it isn't listed among the main agents yet. */
export function isMainChatRow(msg: ChatMessage, selected: string, mainAgentIds: readonly string[] = []): boolean {
  const from = normalizeAgentKey(msg.from || msg.role);
  const to = normalizeAgentKey(msg.to || '');
  if (from === 'system' || from === 'compaction' || from === 'assistant') return true;
  if (from === selected || to === selected) return true;
  if (mainAgentIds.includes(from) || mainAgentIds.includes(to)) return true;
  return (msg.role === 'user' || from === 'user') && !to;
}

const involves = (msg: ChatMessage, id: string) =>
  normalizeAgentKey(msg.from || msg.role) === id || normalizeAgentKey(msg.to || '') === id;

function matchesFilter(msg: ChatMessage, q: string): boolean {
  return (
    msg.text.toLowerCase().includes(q) ||
    normalizeAgentKey(msg.from || msg.role).includes(q) ||
    normalizeAgentKey(msg.to || '').includes(q)
  );
}

export function useChatFilters(opts: {
  chatMessages: ChatMessage[];
  queuedMessages: QueuedChatItem[];
  selectedAgent: string;
  mainAgentIds: readonly string[];
  subagents: SubagentInfo[];
  openSubagentId: string | null;
  subagentMessageFilter: string;
}) {
  const { chatMessages, queuedMessages, selectedAgent, mainAgentIds, subagents, openSubagentId, subagentMessageFilter } = opts;
  const isMissionSession = useSessionStore((s) => s.isMissionSession);
  const selected = normalizeAgentKey(selectedAgent);

  // Mission sessions show all messages — no agent filtering.
  const filteredMainMessages = useMemo(() => {
    const visible = isMissionSession ? chatMessages : chatMessages.filter((m) => isMainChatRow(m, selected, mainAgentIds));
    return collapseProgressMessages(sortMessagesByTime(visible));
  }, [chatMessages, selected, mainAgentIds, isMissionSession]);

  // The finished part keeps its identity while its rows do, so the list's
  // memo holds for the whole stream.
  const { rawHistorical, streamingMessage } = useMemo(() => {
    const len = filteredMainMessages.length;
    const streaming = len > 0 && filteredMainMessages[len - 1].isGenerating;
    return {
      rawHistorical: streaming ? filteredMainMessages.slice(0, len - 1) : filteredMainMessages,
      streamingMessage: streaming ? filteredMainMessages[len - 1] : null,
    };
  }, [filteredMainMessages]);
  const historicalMessages = useStableArray(rawHistorical);

  // Every member's queued messages: the session's turns run one at a time.
  const visibleQueued = useMemo(
    () => queuedMessages.filter((item) => {
      const agent = normalizeAgentKey(item.agent_id);
      return agent === selected || mainAgentIds.includes(agent);
    }),
    [queuedMessages, selected, mainAgentIds],
  );

  const selectedSubagent = useMemo(
    () => subagents.find((sub) => sub.id === openSubagentId) || null,
    [subagents, openSubagentId],
  );
  const filteredSubagentMessages = useMemo(() => {
    if (!selectedSubagent) return [];
    const id = normalizeAgentKey(selectedSubagent.id);
    const mine = sortMessagesByTime(chatMessages.filter((m) => involves(m, id)));
    const q = subagentMessageFilter.trim().toLowerCase();
    return q ? mine.filter((m) => matchesFilter(m, q)) : mine;
  }, [chatMessages, selectedSubagent, subagentMessageFilter]);

  return { filteredMainMessages, historicalMessages, streamingMessage, visibleQueued, selectedSubagent, filteredSubagentMessages };
}

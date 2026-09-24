// Which messages the panel shows: the selected agent's conversation (split
// into a stable finished part and the streaming row), its queue, and the
// open subagent drawer's messages.
import { useMemo } from 'react';
import type { ChatMessage, QueuedChatItem, SubagentInfo } from '../../types';
import { useSessionStore } from '../../stores/sessionStore';
import { useStableArray } from '../../hooks/useStableArray';
import { normalizeAgentKey, sortMessagesByTime, collapseProgressMessages } from './utils/message';

/** A main-chat row for `selected`: its own traffic, the user's to it, and
 *  unrouted notices (system, compaction, skill-page `assistant` rows). */
function isForAgent(msg: ChatMessage, selected: string): boolean {
  const from = normalizeAgentKey(msg.from || msg.role);
  const to = normalizeAgentKey(msg.to || '');
  if (from === 'system' || from === 'compaction' || from === 'assistant') return true;
  if (msg.role === 'user') return !to || to === selected;
  if (from === selected || to === selected) return true;
  if (from === 'user') return to === selected;
  return false;
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
  subagents: SubagentInfo[];
  openSubagentId: string | null;
  subagentMessageFilter: string;
}) {
  const { chatMessages, queuedMessages, selectedAgent, subagents, openSubagentId, subagentMessageFilter } = opts;
  const isMissionSession = useSessionStore((s) => s.isMissionSession);
  const selected = normalizeAgentKey(selectedAgent);

  // Mission sessions show all messages — no agent filtering.
  const filteredMainMessages = useMemo(() => {
    const visible = isMissionSession ? chatMessages : chatMessages.filter((m) => isForAgent(m, selected));
    return collapseProgressMessages(sortMessagesByTime(visible));
  }, [chatMessages, selected, isMissionSession]);

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

  const visibleQueued = useMemo(
    () => queuedMessages.filter((item) => normalizeAgentKey(item.agent_id) === selected),
    [queuedMessages, selected],
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

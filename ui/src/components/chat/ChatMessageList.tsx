// The message rows: one memoized row per message, and the memoized list of
// finished messages (the streaming row renders beside it, so a token only
// re-renders that one row).
import React, { useCallback, useMemo } from 'react';
import { cn } from '../../lib/cn';
import { UNSPOKEN_SENDERS } from '../../lib/messageUtils';
import { useUserStore } from '../../stores/userStore';
import type { ChatMessage } from '../../types';
import { getMessagePhase } from './MessagePhase';
import { AgentMessage } from './AgentMessage';
import { MemoryRecallMessage } from './MemoryRecallMessage';
import { CompactionMessage } from './CompactionMessage';

/** Plumbing rows — context, not a speaker, so never labelled. */
const UNSPOKEN = UNSPOKEN_SENDERS;

/**
 * The bracket label for an agent's message. Every speaking turn carries its
 * name: the panel's own agent as well as anyone relayed in from the phone or
 * via agent_chat. Derived from the message's from_id (one fact, every
 * surface); null only for rows nobody spoke.
 */
function agentLabel(msg: ChatMessage, panelAgent: string): string | null {
  const from = (msg.from || '').toLowerCase();
  if (from === 'user' || UNSPOKEN.has(from) || from.startsWith('run-')) return null;
  // Empty or 'assistant' from_id is this panel's agent speaking — the rows
  // that predate the metadata, and the ones the UI itself synthesizes.
  const speaker = !from || from === 'assistant' ? panelAgent.toLowerCase() : from;
  const label = speaker.charAt(0).toUpperCase() + speaker.slice(1);
  // Rows persisted before the metadata cutover carry the label in the text.
  if (msg.text.startsWith(`[${label}]`)) return null;
  return label;
}

/** Render a single message row. */
export const ChatMessageRow = React.memo<{
  msg: ChatMessage;
  msgKey: string;
  isUser: boolean;
  senderTag?: string | null;
  isExpanded: boolean;
  /** Absent for the streaming row: it has nothing to expand yet. */
  onToggle?: (msgKey: string) => void;
  userMsgIndex?: number;
  userMsgRefs?: React.RefObject<Map<number, HTMLDivElement>>;
  planProps: {
    pendingPlanAgentId?: string | null;
    agentContext?: Record<string, { tokens: number; messages: number; tokenLimit?: number }>;
    onApprovePlan?: () => void;
    onRejectPlan?: () => void;
    onEditPlan?: (text: string) => void;
    inputRef: React.RefObject<HTMLTextAreaElement | null>;
  };
}>(({ msg, msgKey, isUser, senderTag, isExpanded, onToggle, userMsgIndex, userMsgRefs, planProps }) => {
  const toggle = useCallback(() => onToggle?.(msgKey), [onToggle, msgKey]);
  const registerRef = useCallback((el: HTMLDivElement | null) => {
    if (userMsgIndex == null || !userMsgRefs?.current) return;
    if (el) userMsgRefs.current.set(userMsgIndex, el);
    else userMsgRefs.current.delete(userMsgIndex);
  }, [userMsgIndex, userMsgRefs]);
  // Auto-recall messages get their own collapsible chip. Backend
  // persists them with from_id="memory-recall" (runtime.rs::
  // push_user_turn_with_recall). Legacy sessions used from_id="memory",
  // which collides with the memory *agent's* id (its dream-mission
  // replies rendered as recall chips) — so the legacy match also
  // requires the recall text shape.
  if (msg.from === 'memory-recall' || (msg.from === 'memory' && msg.text.startsWith('From memory'))) {
    return <MemoryRecallMessage text={msg.text} />;
  }
  // Auto-compaction notice, injected by handleContextUsage on `compressed`.
  if (msg.from === 'compaction') {
    return <CompactionMessage text={msg.text} />;
  }
  const phase = isUser ? undefined : getMessagePhase(msg);
  const messageClass = isUser
    ? 'bg-slate-100 dark:bg-white/10 text-slate-900 dark:text-slate-100 rounded-md px-2.5 py-1.5'
    : phase === 'thinking'
      ? ''
      : msg.isThinking && !msg.isGenerating
        ? 'text-slate-500 dark:text-slate-400 italic opacity-60'
        : 'text-slate-800 dark:text-slate-200';
  return (
    <div
      key={msgKey}
      ref={userMsgIndex != null ? registerRef : undefined}
      className={cn('w-full flex', isUser ? 'justify-end' : 'justify-start')}
    >
      {/* A typed message keeps the shape it was typed in: line breaks and
          indentation are how a list, an address block or a pasted email
          reads. Rendering it raw collapsed all of it into one paragraph. */}
      <div className={cn(isUser ? 'max-w-[96%] whitespace-pre-wrap break-words' : 'max-w-full', 'text-[14px] leading-relaxed', messageClass)}>
        {senderTag && (
          <span className="font-semibold text-emerald-600 dark:text-emerald-400 mr-1.5">
            [{senderTag}]
          </span>
        )}
        {isUser ? (
          <>
            {msg.text}
            {(msg.imageCount ?? msg.images?.length ?? 0) > 0 && (
              <span className="text-slate-400 dark:text-slate-500 ml-1">
                {(() => {
                  const count = msg.imageCount ?? msg.images?.length ?? 0;
                  return Array.from({ length: count }, (_, i) => `[image${count > 1 ? `#${i + 1}` : ''}]`).join(' ');
                })()}
              </span>
            )}
          </>
        ) : (
          <AgentMessage msg={msg} isExpanded={isExpanded} onToggle={toggle} planProps={planProps} />
        )}
      </div>
    </div>
  );
});

/** Memoized historical message list — skips re-render during streaming & typing. */
export const ChatMessageList = React.memo<{
  messages: ChatMessage[];
  expandedMessages: Set<string>;
  setExpandedMessages: React.Dispatch<React.SetStateAction<Set<string>>>;
  verboseMode?: boolean;
  userMsgRefs: React.RefObject<Map<number, HTMLDivElement>>;
  selectedAgent: string;
  pendingPlanAgentId?: string | null;
  agentContext?: Record<string, { tokens: number; messages: number; tokenLimit?: number }>;
  onApprovePlan?: () => void;
  onRejectPlan?: () => void;
  onEditPlan?: (text: string) => void;
  inputRef: React.RefObject<HTMLTextAreaElement | null>;
}>(({ messages, expandedMessages, setExpandedMessages, verboseMode, userMsgRefs, selectedAgent, pendingPlanAgentId, agentContext, onApprovePlan, onRejectPlan, onEditPlan, inputRef }) => {
  const planProps = useMemo(() => ({ pendingPlanAgentId, agentContext, onApprovePlan, onRejectPlan, onEditPlan, inputRef }), [pendingPlanAgentId, agentContext, onApprovePlan, onRejectPlan, onEditPlan, inputRef]);
  // The user's name, as core memory states it — labels their bubbles. Until
  // the real name is learned, the placeholder "Hanli" stands in (Yinyue's
  // persona explains it and asks for the real one).
  const coreName = useUserStore((s) => s.coreName);
  // One toggle for every row (keyed by the row's key), so a row's props
  // stay equal across renders and its memo holds.
  const toggleExpanded = useCallback((key: string) => {
    setExpandedMessages((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, [setExpandedMessages]);
  return (
    <>
      {messages.length === 0 && (
        <div className="self-center mt-12 max-w-md text-center">
          <div className="text-sm font-semibold text-slate-600 dark:text-slate-300">
            No messages for {selectedAgent}
          </div>
          <div className="mt-2 text-xs text-slate-500">
            Send a message to this main agent or switch tabs.
          </div>
        </div>
      )}
      {messages.map((msg, i) => {
        // Skip hidden system messages (used by app skills for internal prompts)
        if (msg.role === 'user' && msg.text.startsWith('[HIDDEN]')) return null;
        const key = `${msg.timestamp}-${i}-${msg.from || msg.role}-${msg.text.slice(0, 24)}`;
        const isUser = msg.role === 'user';
        const isExpanded = verboseMode || expandedMessages.has(key);
        const userMsgIndex = isUser ? i : undefined;
        return (
          <ChatMessageRow
            key={key}
            msg={msg}
            msgKey={key}
            isUser={isUser}
            senderTag={
              isUser && (!msg.from || msg.from === 'user')
                ? (coreName ?? 'Hanli')
                : agentLabel(msg, selectedAgent)
            }
            isExpanded={isExpanded}
            onToggle={toggleExpanded}
            userMsgIndex={userMsgIndex}
            userMsgRefs={userMsgRefs}
            planProps={planProps}
          />
        );
      })}
    </>
  );
});

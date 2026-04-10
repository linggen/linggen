import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Sparkles, ArrowDown } from 'lucide-react';
import 'highlight.js/styles/github.css';
import { cn } from '../../lib/cn';
import { useProjectStore } from '../../stores/projectStore';
import { useAgentStore } from '../../stores/agentStore';
import { AskUserCard } from '../AskUserCard';
import { ToolPermissionCard } from '../ToolPermissionCard';
import type {
  AgentInfo,
  ChatMessage,
  QueuedChatItem,
  SkillInfo,
  SubagentInfo,
} from '../../types';
import { normalizeAgentKey, sortMessagesByTime, collapseProgressMessages } from './utils/message';
import { getMessagePhase } from './MessagePhase';
import { AgentMessage } from './AgentMessage';
import { ChatInput } from './ChatInput';
import { SubagentDrawer } from './SubagentDrawer';
import { statusBadgeClass } from './MessageHelpers';
import { SessionModelSelector, SessionModeSelector, SessionStats } from './SessionSelectors';

/** Render a single message row. */
const ChatMessageRow = React.memo<{
  msg: ChatMessage;
  msgKey: string;
  isUser: boolean;
  isExpanded: boolean;
  onToggle: () => void;
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
}>(({ msg, msgKey, isUser, isExpanded, onToggle, userMsgIndex, userMsgRefs, planProps }) => {
  const registerRef = useCallback((el: HTMLDivElement | null) => {
    if (userMsgIndex == null || !userMsgRefs?.current) return;
    if (el) userMsgRefs.current.set(userMsgIndex, el);
    else userMsgRefs.current.delete(userMsgIndex);
  }, [userMsgIndex, userMsgRefs]);
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
      <div className={cn(isUser ? 'max-w-[96%]' : 'max-w-full', 'text-[14px] leading-relaxed', messageClass)}>
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
          <AgentMessage msg={msg} isExpanded={isExpanded} onToggle={onToggle} planProps={planProps} />
        )}
      </div>
    </div>
  );
});

/** Memoized historical message list — skips re-render during streaming & typing. */
const ChatMessageList = React.memo<{
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
            isExpanded={isExpanded}
            onToggle={() => {
              setExpandedMessages((prev) => {
                const next = new Set(prev);
                if (next.has(key)) next.delete(key);
                else next.add(key);
                return next;
              });
            }}
            userMsgIndex={userMsgIndex}
            userMsgRefs={userMsgRefs}
            planProps={planProps}
          />
        );
      })}
    </>
  );
});
export const ChatPanel: React.FC<{
  chatMessages: ChatMessage[];
  queuedMessages: QueuedChatItem[];
  chatEndRef: React.RefObject<HTMLDivElement | null>;
  projectRoot?: string | null;
  sessionId?: string | null;
  selectedAgent: string;
  setSelectedAgent: (value: string) => void;
  skills: SkillInfo[];
  agents: AgentInfo[];
  mainAgents: AgentInfo[];
  subagents: SubagentInfo[];
  runningMainRunIds?: Record<string, string>;
  cancellingRunIds?: Record<string, boolean>;
  onCancelRun?: (runId: string) => void | Promise<void>;
  onSendMessage: (message: string, targetAgent?: string, images?: string[]) => void;
  activePlan?: import('../../types').Plan | null;
  pendingPlan?: import('../../types').Plan | null;
  pendingPlanAgentId?: string | null;
  agentContext?: Record<string, { tokens: number; messages: number; tokenLimit?: number }>;
  onApprovePlan?: () => void;
  onRejectPlan?: () => void;
  onEditPlan?: (text: string) => void;
  pendingAskUser?: import('../../types').PendingAskUser | null;
  onRespondToAskUser?: (questionId: string, answers: import('../../types').AskUserAnswer[]) => void;
  onCancelAgentRun?: (runId: string) => void | Promise<void>;
  isRunning?: boolean;
  verboseMode?: boolean;
  agentStatus?: Record<string, string>;
  overlay?: string | null;
  onDismissOverlay?: () => void;
  modelPickerOpen?: boolean;
  models?: import('../../types').ModelInfo[];
  defaultModels?: string[];
  onSwitchModel?: (modelId: string) => void;
  tokensPerSec?: number;
  mobile?: boolean;
  scrollToBottom?: () => void;
  showScrollButton?: boolean;
}> = ({
  chatMessages,
  queuedMessages,
  chatEndRef,
  projectRoot,
  sessionId,
  selectedAgent,
  setSelectedAgent,
  skills,
  agents,
  mainAgents,
  subagents,
  runningMainRunIds,
  cancellingRunIds,
  onCancelRun,
  onSendMessage,
  activePlan,
  pendingPlan: _pendingPlan,
  pendingPlanAgentId,
  agentContext,
  onApprovePlan,
  onRejectPlan,
  onEditPlan,
  pendingAskUser,
  onRespondToAskUser,
  onCancelAgentRun,
  isRunning,
  verboseMode,
  agentStatus,
  overlay,
  onDismissOverlay,
  modelPickerOpen,
  models: modelsList,
  defaultModels: defaultModelsList,
  onSwitchModel,
  tokensPerSec,
  mobile,
  scrollToBottom: scrollToBottomProp,
  showScrollButton: showScrollButtonProp,
}) => {
  const [openSubagentId, setOpenSubagentId] = useState<string | null>(null);
  const [mainMessageFilter, setMainMessageFilter] = useState('');
  const [subagentMessageFilter, setSubagentMessageFilter] = useState('');

  const [expandedMessages, setExpandedMessages] = useState<Set<string>>(new Set());
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const chatScrollRef = useRef<HTMLDivElement | null>(null);
  const userMsgRefs = useRef<Map<number, HTMLDivElement>>(new Map());
  const [floatingUserMsg, setFloatingUserMsg] = useState<{ index: number; text: string } | null>(null);
  const thinkingStartRef = useRef<number | null>(null);
  const [thinkingElapsed, setThinkingElapsed] = useState(0);

  useEffect(() => {
    if (pendingAskUser) {
      chatEndRef?.current?.scrollIntoView({ behavior: 'auto', block: 'nearest' });
    }
  }, [pendingAskUser, chatEndRef]);

  // Auto-scroll is handled by useAutoScroll hook in ChatWidget.
  // We just receive scrollToBottom and showScrollButton as props.
  const showScrollButton = showScrollButtonProp ?? false;
  const scrollToBottom = scrollToBottomProp ?? (() => {
    chatEndRef?.current?.scrollIntoView({ behavior: 'smooth', block: 'end' });
  });

  // Track agent active state and elapsed time — keyed by session ID
  const agentStatusText = useAgentStore((s) => s.agentStatusText);
  const currentStatus = agentStatus?.[sessionId || ''];
  const isAgentActive = !!currentStatus && currentStatus !== 'idle';
  const _isThinking = currentStatus === 'thinking' || currentStatus === 'model_loading';
  const [spinnerVerb, setSpinnerVerb] = useState('');
  const [lastRunSummary, setLastRunSummary] = useState<{ verb: string; elapsed: number } | null>(null);
  useEffect(() => {
    if (isAgentActive) {
      if (!thinkingStartRef.current) {
        thinkingStartRef.current = Date.now();
        const verbs = [
          'Thinking', 'Pondering', 'Brewing', 'Cogitating', 'Reticulating',
          'Noodling', 'Musing', 'Simmering', 'Percolating', 'Ruminating',
          'Contemplating', 'Marinating', 'Conjuring', 'Scheming', 'Tinkering',
          'Crafting', 'Hatching', 'Computing', 'Deliberating',
        ];
        setSpinnerVerb(verbs[Math.floor(Math.random() * verbs.length)]);
        setLastRunSummary(null);
      }
      const interval = setInterval(() => {
        setThinkingElapsed(Math.floor((Date.now() - (thinkingStartRef.current || Date.now())) / 1000));
      }, 500);
      return () => clearInterval(interval);
    } else {
      if (thinkingStartRef.current) {
        const elapsed = Math.floor((Date.now() - thinkingStartRef.current) / 1000);
        if (elapsed > 0) setLastRunSummary({ verb: spinnerVerb || 'Worked', elapsed });
      }
      thinkingStartRef.current = null;
      setThinkingElapsed(0);
    }
  }, [isAgentActive]); // eslint-disable-line react-hooks/exhaustive-deps

  const mainAgentIds = useMemo(
    () => mainAgents.map((agent) => normalizeAgentKey(agent.name)),
    [mainAgents]
  );

  const isMissionSession = useProjectStore((s) => s.isMissionSession);

  const visibleMessages = useMemo(() => {
    // Mission sessions show all messages — no agent filtering needed.
    if (isMissionSession) return chatMessages;

    const selected = normalizeAgentKey(selectedAgent);
    return chatMessages.filter((msg) => {
      const from = normalizeAgentKey(msg.from || msg.role);
      const to = normalizeAgentKey(msg.to || '');
      // Always show system messages (e.g. /status, /help output)
      if (from === 'system') return true;
      if (msg.role === 'user') {
        return !to || to === selected;
      }
      if (from === selected || to === selected) return true;
      if (from === 'user') return to === selected;
      return false;
    });
  }, [chatMessages, selectedAgent, isMissionSession]);

  const visibleQueued = useMemo(
    () => queuedMessages.filter((item) => normalizeAgentKey(item.agent_id) === normalizeAgentKey(selectedAgent)),
    [queuedMessages, selectedAgent]
  );

  const selectedSubagent = useMemo(
    () => subagents.find((sub) => sub.id === openSubagentId) || null,
    [subagents, openSubagentId]
  );
  const selectedAgentKey = normalizeAgentKey(selectedAgent);
  const selectedMainRunningRunId = runningMainRunIds?.[selectedAgentKey];
  const selectedSubagentKey = selectedSubagent ? normalizeAgentKey(selectedSubagent.id) : '';
  const subagentMessages = useMemo(() => {
    if (!selectedSubagent) return [];
    const id = normalizeAgentKey(selectedSubagent.id);
    const filtered = chatMessages.filter((msg) => {
      const from = normalizeAgentKey(msg.from || msg.role);
      const to = normalizeAgentKey(msg.to || '');
      return from === id || to === id;
    });
    return sortMessagesByTime(filtered);
  }, [chatMessages, selectedSubagent]);
  const displayedMainMessages = useMemo(
    () => collapseProgressMessages(sortMessagesByTime(visibleMessages)),
    [visibleMessages]
  );
  const filteredMainMessages = useMemo(() => {
    const q = mainMessageFilter.trim().toLowerCase();
    if (!q) return displayedMainMessages;
    return displayedMainMessages.filter((msg) => {
      const from = normalizeAgentKey(msg.from || msg.role);
      const to = normalizeAgentKey(msg.to || '');
      const activitySummary = (msg.activitySummary || '').toLowerCase();
      const activityLines = (msg.activityEntries || []).join('\n').toLowerCase();
      return (
        msg.text.toLowerCase().includes(q) ||
        activitySummary.includes(q) ||
        activityLines.includes(q) ||
        from.includes(q) ||
        to.includes(q)
      );
    });
  }, [displayedMainMessages, mainMessageFilter]);

  // Show floating banner with nearest user message scrolled above viewport
  const filteredMainMessagesRef = useRef(filteredMainMessages);
  filteredMainMessagesRef.current = filteredMainMessages;
  useEffect(() => {
    const container = chatScrollRef.current;
    if (!container) return;
    let rafId = 0;
    const update = () => {
      const containerTop = container.getBoundingClientRect().top;
      const threshold = containerTop + 48; // account for floating bar height
      let bestIdx = -1;
      let bestTop = -Infinity;
      for (const [idx, el] of userMsgRefs.current.entries()) {
        const top = el.getBoundingClientRect().top;
        if (top < threshold && top > bestTop) {
          bestTop = top;
          bestIdx = idx;
        }
      }
      if (bestIdx >= 0) {
        const msg = filteredMainMessagesRef.current[bestIdx];
        setFloatingUserMsg((prev) => {
          if (prev?.index === bestIdx && prev?.text === msg?.text) return prev;
          return msg ? { index: bestIdx, text: msg.text } : null;
        });
      } else {
        setFloatingUserMsg((prev) => prev === null ? prev : null);
      }
    };
    const onScroll = () => {
      cancelAnimationFrame(rafId);
      rafId = requestAnimationFrame(update);
    };
    container.addEventListener('scroll', onScroll, { passive: true });
    update(); // initial check
    return () => {
      container.removeEventListener('scroll', onScroll);
      cancelAnimationFrame(rafId);
    };
  }, []);

  // Split messages: historical (stable, memoized) vs streaming (re-renders per token)
  const { historicalMessages, streamingMessage } = useMemo(() => {
    const len = filteredMainMessages.length;
    if (len > 0 && filteredMainMessages[len - 1].isGenerating) {
      return {
        historicalMessages: filteredMainMessages.slice(0, len - 1),
        streamingMessage: filteredMainMessages[len - 1],
      };
    }
    return { historicalMessages: filteredMainMessages, streamingMessage: null };
  }, [filteredMainMessages]);

  const filteredSubagentMessages = useMemo(() => {
    const q = subagentMessageFilter.trim().toLowerCase();
    if (!q) return subagentMessages;
    return subagentMessages.filter((msg) => {
      const from = normalizeAgentKey(msg.from || msg.role);
      const to = normalizeAgentKey(msg.to || '');
      return (
        msg.text.toLowerCase().includes(q) ||
        from.includes(q) ||
        to.includes(q)
      );
    });
  }, [subagentMessages, subagentMessageFilter]);

  useEffect(() => {
    if (!openSubagentId) return;
    if (!subagents.some((sub) => sub.id === openSubagentId)) {
      setOpenSubagentId(null);
    }
  }, [openSubagentId, subagents]);

  return (
    <section className="h-full flex flex-col bg-white dark:bg-[#0f0f0f] rounded-xl border border-slate-200 dark:border-white/5 overflow-hidden min-h-0 relative">
      <div className="px-1.5 py-1 border-b border-slate-200 dark:border-white/5 bg-slate-50/70 dark:bg-white/[0.02] space-y-1">
        {sessionId && (
          <details className="rounded-md border border-slate-200 dark:border-white/10 bg-white/80 dark:bg-black/20 px-2 py-1 text-[11px] text-slate-600 dark:text-slate-300">
            <summary className="cursor-pointer flex flex-wrap items-center gap-2">
              {sessionId && (() => {
                const sessionMeta = useProjectStore.getState().allSessions.find(s => s.id === sessionId);
                return (
                  <>
                    <span className="font-semibold uppercase tracking-wider text-slate-500">Session</span>
                    <span className="font-mono truncate max-w-[160px]">{sessionId}</span>
                    {(sessionMeta?.project_name || sessionMeta?.cwd) && (() => {
                      const fullPath = sessionMeta?.cwd || sessionMeta?.project || '';
                      const displayName = sessionMeta?.project_name || fullPath.split('/').filter(Boolean).pop() || fullPath;
                      return (
                        <span className="px-1.5 py-0.5 rounded bg-slate-100 dark:bg-white/5 text-slate-500 text-[10px] shrink-0" title={fullPath}>
                          📁 {displayName}
                        </span>
                      );
                    })()}
                    {sessionMeta?.creator && sessionMeta.creator !== 'user' && (
                      <span className={cn('px-1.5 py-0.5 rounded text-[10px] font-medium',
                        sessionMeta.creator === 'mission' ? 'bg-amber-100 dark:bg-amber-500/10 text-amber-600 dark:text-amber-400'
                          : 'bg-purple-100 dark:bg-purple-500/10 text-purple-600 dark:text-purple-400'
                      )}>
                        {sessionMeta.creator === 'mission' ? '🤖' : '✨'} {sessionMeta.creator}
                      </span>
                    )}
                    {sessionMeta?.skill && sessionMeta.creator !== 'mission' && (
                      <span className="px-1.5 py-0.5 rounded bg-purple-50 dark:bg-purple-500/5 text-purple-500 text-[10px]">
                        {sessionMeta.skill}
                      </span>
                    )}
                  </>
                );
              })()}
              <SessionModeSelector />
              <SessionModelSelector />
              <SessionStats />
            </summary>
            <div className="mt-1.5 flex flex-wrap items-center gap-2">
              {selectedMainRunningRunId && onCancelRun && (
                <button
                  onClick={() => onCancelRun(selectedMainRunningRunId)}
                  disabled={!!cancellingRunIds?.[selectedMainRunningRunId]}
                  className={cn(
                    'px-2 py-1 rounded border text-[11px] font-semibold transition-colors',
                    cancellingRunIds?.[selectedMainRunningRunId]
                      ? 'bg-slate-100 text-slate-400 border-slate-200 cursor-not-allowed'
                      : 'bg-red-50 text-red-600 border-red-200 hover:bg-red-100'
                  )}
                  title={selectedMainRunningRunId}
                >
                  {cancellingRunIds?.[selectedMainRunningRunId] ? 'Cancelling...' : 'Cancel Run'}
                </button>
              )}
            </div>
          </details>
        )}

        {subagents.length > 0 && (
          <details className="rounded-md border border-slate-200 dark:border-white/10 bg-white/80 dark:bg-black/20 px-2 py-1 text-[11px]">
            <summary className="cursor-pointer font-semibold uppercase tracking-wider text-slate-500 dark:text-slate-400 flex items-center gap-1">
              <Sparkles size={11} />
              Subagents ({subagents.length})
            </summary>
            <div className="mt-1 flex flex-wrap items-center gap-1.5">
              {subagents.map((sub) => (
                <button
                  key={sub.id}
                  onClick={() => setOpenSubagentId(sub.id)}
                  className={cn(
                    'px-2 py-1 rounded-md text-[11px] border transition-colors flex items-center gap-1',
                    openSubagentId === sub.id
                      ? 'bg-blue-600 text-white border-blue-600'
                      : 'bg-slate-100 dark:bg-white/5 border-slate-200 dark:border-white/10 hover:bg-slate-200 dark:hover:bg-white/10'
                  )}
                >
                  <span className="font-semibold">{sub.id}</span>
                  <span className={cn('px-1.5 py-0.5 rounded-full uppercase tracking-wide', statusBadgeClass(sub.status))}>
                    {sub.status}
                  </span>
                </button>
              ))}
            </div>
          </details>
        )}
      </div>

      <div ref={chatScrollRef} className="relative flex-1 overflow-y-scroll px-2 py-1.5 flex flex-col gap-2 custom-scrollbar min-h-0">
        {floatingUserMsg && (
          <div
            className="sticky top-0 z-20 mx-1 mb-1 px-3 py-2 rounded-md bg-slate-100/95 dark:bg-white/10 backdrop-blur text-[13px] text-slate-700 dark:text-slate-200 border border-slate-200/60 dark:border-white/10 cursor-pointer"
            onClick={() => {
              const el = userMsgRefs.current.get(floatingUserMsg.index);
              el?.scrollIntoView({ behavior: 'smooth', block: 'center' });
            }}
            title={floatingUserMsg.text}
          >
            <p className="line-clamp-4 whitespace-pre-wrap break-words">
              <span className="font-medium text-slate-500 dark:text-slate-400 mr-1.5">You:</span>
              {floatingUserMsg.text}
            </p>
          </div>
        )}
        <div className="flex items-center justify-end gap-2 mb-1">
          <input
            value={mainMessageFilter}
            onChange={(e) => setMainMessageFilter(e.target.value)}
            placeholder="Filter messages"
            className="w-52 text-[12px] bg-white dark:bg-black/30 border border-slate-200 dark:border-white/10 rounded px-2 py-1 outline-none"
          />
        </div>
        <ChatMessageList
          messages={historicalMessages}
          expandedMessages={expandedMessages}
          setExpandedMessages={setExpandedMessages}
          verboseMode={verboseMode}
          userMsgRefs={userMsgRefs}
          selectedAgent={selectedAgent}
          pendingPlanAgentId={pendingPlanAgentId}
          agentContext={agentContext}
          onApprovePlan={onApprovePlan}
          onRejectPlan={onRejectPlan}
          onEditPlan={onEditPlan}
          inputRef={inputRef}
        />
        {streamingMessage && (
          <ChatMessageRow
            msg={streamingMessage}
            msgKey={`${streamingMessage.timestamp}-${filteredMainMessages.length - 1}-${streamingMessage.from || streamingMessage.role}-${streamingMessage.text.slice(0, 24)}`}
            isUser={false}
            isExpanded={verboseMode || false}
            onToggle={() => {}}
            planProps={{ pendingPlanAgentId, agentContext, onApprovePlan, onRejectPlan, onEditPlan, inputRef }}
          />
        )}
        {pendingAskUser && onRespondToAskUser && (
          <div className="px-3 py-2">
            {pendingAskUser.questions[0]?.header === 'Permission'
              ? <ToolPermissionCard pending={pendingAskUser} onRespond={onRespondToAskUser} />
              : <AskUserCard pending={pendingAskUser} onRespond={onRespondToAskUser} />}
          </div>
        )}
        <div ref={chatEndRef} />
        {showScrollButton && (
          <button
            onClick={scrollToBottom}
            className="sticky bottom-2 left-1/2 -translate-x-1/2 z-30 w-8 h-8 rounded-full bg-white dark:bg-[#1a1a1a] border border-slate-300 dark:border-white/15 shadow-lg flex items-center justify-center hover:bg-slate-50 dark:hover:bg-white/10 transition-all opacity-80 hover:opacity-100"
            title="Scroll to bottom"
          >
            <ArrowDown size={14} className="text-slate-600 dark:text-slate-300" />
          </button>
        )}
      </div>

      {/* Status spinner — always visible when active or just completed */}
      {isAgentActive ? (
        <div className="px-3 py-1.5">
          <div className="flex items-center gap-1.5 text-[13px] text-slate-500 dark:text-slate-400 font-medium animate-pulse">
            <span className="text-blue-500">✶</span>
            <span>
              {agentStatusText?.[sessionId || ''] || (currentStatus === 'model_loading' ? 'Loading model' : spinnerVerb || 'Thinking')}…
              {(thinkingElapsed > 0 || (agentContext?.[sessionId || '']?.tokens ?? 0) > 0) && (
                <span className="font-normal text-slate-400 dark:text-slate-500 ml-1">
                  ({[
                    thinkingElapsed >= 60
                      ? `${Math.floor(thinkingElapsed / 60)}m ${thinkingElapsed % 60}s`
                      : thinkingElapsed > 0 ? `${thinkingElapsed}s` : '',
                    (tokensPerSec ?? 0) > 0
                      ? `${tokensPerSec!.toFixed(1)} tok/s`
                      : '',
                    (agentContext?.[sessionId || '']?.tokens ?? 0) > 0
                      ? (() => {
                          const t = agentContext?.[sessionId || '']?.tokens ?? 0;
                          const lim = agentContext?.[sessionId || '']?.tokenLimit;
                          const tk = `${(t / 1000).toFixed(1)}k`;
                          return lim ? `${tk}/${lim >= 1_000_000 ? `${(lim / 1_000_000).toFixed(lim % 1_000_000 === 0 ? 0 : 1)}M` : `${Math.round(lim / 1000)}K`} ctx (${Math.round(t / lim * 100)}%)` : `${tk} ctx`;
                        })()
                      : '',
                  ].filter(Boolean).join(' · ')})
                </span>
              )}
            </span>
          </div>
        </div>
      ) : lastRunSummary && (
        <div className="px-3 py-1.5">
          <div className="flex items-center gap-1.5 text-[13px] text-slate-400 dark:text-slate-500 italic">
            <span>✻</span>
            <span>
              {lastRunSummary.verb} for{' '}
              {lastRunSummary.elapsed >= 60
                ? `${Math.floor(lastRunSummary.elapsed / 60)}m ${lastRunSummary.elapsed % 60}s`
                : `${lastRunSummary.elapsed}s`}
            </span>
          </div>
        </div>
      )}
      <ChatInput
        projectRoot={projectRoot}
        selectedAgent={selectedAgent}
        setSelectedAgent={setSelectedAgent}
        skills={skills}
        agents={agents}
        mainAgentIds={mainAgentIds}
        isRunning={isRunning}
        onSendMessage={onSendMessage}
        onCancelAgentRun={onCancelAgentRun}
        selectedMainRunningRunId={selectedMainRunningRunId}
        activePlan={activePlan}
        visibleQueued={visibleQueued}
        overlay={overlay}
        onDismissOverlay={onDismissOverlay}
        inputRef={inputRef}
        modelPickerOpen={modelPickerOpen}
        models={modelsList}
        defaultModels={defaultModelsList}
        onSwitchModel={onSwitchModel}
        mobile={mobile}
      />

      {selectedSubagent && (
        <SubagentDrawer
          selectedSubagent={selectedSubagent}
          filteredSubagentMessages={filteredSubagentMessages}
          subagentMessageFilter={subagentMessageFilter}
          setSubagentMessageFilter={setSubagentMessageFilter}
          cancellingRunIds={cancellingRunIds}
          onCancelRun={onCancelRun}
          onClose={() => setOpenSubagentId(null)}
        />
      )}
    </section>
  );
};

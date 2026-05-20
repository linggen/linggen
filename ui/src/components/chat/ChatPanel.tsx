import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Sparkles, ArrowDown, Copy, FileText, Eraser } from 'lucide-react';
import 'highlight.js/styles/github.css';
import { cn } from '../../lib/cn';
import { useSessionStore } from '../../stores/sessionStore';
import { useServerStore } from '../../stores/serverStore';
import { useUserStore } from '../../stores/userStore';
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
import { SubagentPane } from './SubagentPane';
import { statusBadgeClass } from './MessageHelpers';
import { SessionModelSelector, SessionModeSelector, SessionStats } from './SessionSelectors';
import { useChatActions } from '../../hooks/useChatActions';

/**
 * Debug action buttons shown inside the expanded session header.
 * Hidden by default — only visible when the user clicks the session bar open.
 */
const ChatDebugActions: React.FC<{ projectRoot?: string | null; sessionId?: string | null }> = ({ projectRoot, sessionId }) => {
  const [copyStatus, setCopyStatus] = useState<'idle' | 'copied' | 'error'>('idle');
  const [spStatus, setSpStatus] = useState<'idle' | 'copied' | 'error'>('idle');
  const { copyChat, clearChat } = useChatActions(() => {}, {}, projectRoot);

  const handleCopyChat = useCallback(async () => {
    try { await copyChat(); setCopyStatus('copied'); }
    catch { setCopyStatus('error'); }
    setTimeout(() => setCopyStatus('idle'), 1500);
  }, [copyChat]);

  const handleCopySystemPrompt = useCallback(async () => {
    const root = projectRoot || useSessionStore.getState().selectedProjectRoot || '';
    const agentId = useServerStore.getState().selectedAgent;
    const sid = sessionId || useSessionStore.getState().activeSessionId;
    let url: URL | undefined;
    try {
      url = new URL('/api/chat/system-prompt', window.location.origin);
      url.searchParams.append('project_root', root);
      url.searchParams.append('agent_id', agentId);
      if (sid) url.searchParams.append('session_id', sid);
      const resp = await fetch(url.toString());
      if (!resp.ok) {
        const bodyHint = await resp.text().catch(() => '');
        throw new Error(`HTTP ${resp.status}: ${bodyHint.slice(0, 200)}`);
      }
      const payload = await resp.json();
      const promptText = payload.system_prompt || '';
      const tools = Array.isArray(payload.tools) ? payload.tools : [];
      if (!promptText && tools.length === 0) throw new Error('empty payload');
      // Combine the system prompt and the tool schemas so the copy shows the
      // full model-facing surface. Tool schemas travel via the native API
      // `tools` parameter, not the prompt text, but they are part of what the
      // model actually receives.
      const toolsSection = tools.length > 0
        ? `\n\n--- TOOLS (passed via native function calling) ---\n${JSON.stringify(tools, null, 2)}\n--- END TOOLS ---\n`
        : '';
      const text = `${promptText}${toolsSection}`;
      try { await navigator.clipboard.writeText(text); }
      catch (clipErr) {
        // Browser may block clipboard outside a user gesture (e.g. after the
        // await round-trip) — fall back to the textarea+execCommand trick.
        const ta = document.createElement('textarea');
        ta.value = text; ta.style.position = 'fixed'; ta.style.opacity = '0';
        document.body.appendChild(ta); ta.select();
        const ok = document.execCommand('copy');
        document.body.removeChild(ta);
        if (!ok) throw new Error(`clipboard fallback failed: ${(clipErr as Error)?.message ?? 'unknown'}`);
      }
      setSpStatus('copied');
    } catch (err) {
      console.error('[copy-system-prompt] failed:', err, { url: url?.toString() });
      setSpStatus('error');
    }
    setTimeout(() => setSpStatus('idle'), 1500);
  }, [projectRoot, sessionId]);

  const btnClass = (status: 'idle' | 'copied' | 'error') => cn(
    'px-1.5 py-1 rounded transition-colors text-slate-500 dark:text-slate-400 shrink-0 flex items-center gap-1 text-[11px] font-medium',
    status === 'copied' ? 'bg-green-500/10 text-green-600'
      : status === 'error' ? 'bg-red-500/10 text-red-500'
      : 'hover:bg-slate-100 dark:hover:bg-white/5',
  );

  const labelFor = (status: 'idle' | 'copied' | 'error', idle: string) =>
    status === 'copied' ? 'Copied' : status === 'error' ? 'Failed' : idle;

  return (
    <div className="flex items-center gap-1" onClick={(e) => e.stopPropagation()}>
      <button onClick={handleCopyChat} className={btnClass(copyStatus)} title="Copy chat transcript to clipboard">
        <Copy size={12} />
        <span>{labelFor(copyStatus, 'Chat')}</span>
      </button>
      <button onClick={handleCopySystemPrompt} className={btnClass(spStatus)} title="Copy system prompt + tool schemas to clipboard">
        <FileText size={12} />
        <span>{labelFor(spStatus, 'System Prompt')}</span>
      </button>
      <button onClick={clearChat}
        className="px-1.5 py-1 rounded text-slate-500 dark:text-slate-400 hover:bg-red-500/10 hover:text-red-500 transition-colors shrink-0 flex items-center gap-1 text-[11px] font-medium"
        title="Clear chat messages">
        <Eraser size={12} />
        <span>Clear</span>
      </button>
    </div>
  );
};

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
  /** Render the dedicated SubagentPane on the right (or stacked on
   *  narrow screens). Default true at the top-level Linggen UI; iframe
   *  consumers should pass `false` and let the inline SubagentTreeView
   *  inside each parent message handle subagent visibility instead. */
  showSubagentPane?: boolean;
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
  showSubagentPane = true,
}) => {
  const [openSubagentId, setOpenSubagentId] = useState<string | null>(null);
  const [subagentMessageFilter, setSubagentMessageFilter] = useState('');

  const [expandedMessages, setExpandedMessages] = useState<Set<string>>(new Set());
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const chatScrollRef = useRef<HTMLDivElement | null>(null);
  const userMsgRefs = useRef<Map<number, HTMLDivElement>>(new Map());
  const [floatingUserMsg, setFloatingUserMsg] = useState<{ index: number; text: string } | null>(null);
  const thinkingStartRef = useRef<number | null>(null);
  const [thinkingElapsed, setThinkingElapsed] = useState(0);

  // Focus chat input when session changes (e.g. new session created)
  useEffect(() => {
    if (sessionId) {
      // Small delay to let the DOM settle after session switch
      setTimeout(() => inputRef.current?.focus(), 50);
    }
  }, [sessionId]);

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
  const agentStatusText = useServerStore((s) => s.agentStatusText);
  const currentStatus = agentStatus?.[sessionId || ''];
  // Spinner is bound to `isRunning` — the server-authoritative "agent
  // run in flight" signal — NOT to `agentStatus[sid]`. The latter
  // proxy was set from too many paths (handleTurnComplete,
  // handleSubagentResult, applyTopLevelActivity, pageState.busy_sessions, …)
  // and drifted out of sync, leaving the spinner stuck while text said
  // "Idle". When `isRunning` is false there's literally nothing
  // executing for this session — the spinner has no business showing.
  const isAgentActive = !!isRunning;
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

  const isMissionSession = useSessionStore((s) => s.isMissionSession);

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
  const filteredMainMessages = useMemo(
    () => collapseProgressMessages(sortMessagesByTime(visibleMessages)),
    [visibleMessages]
  );

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

  // Subagent pane visibility. Show whenever any message in this session
  // has a subagent tree; auto-collapse 5s after the LAST subagent goes
  // done (status !== 'running') and there are no widgets needing the
  // user. Reopens immediately on the next spawn.
  const subagentEntries = useMemo(() => {
    const out: import('../../types').SubagentTreeEntry[] = [];
    for (const m of filteredMainMessages) {
      if (m.subagentTree && m.subagentTree.length > 0) {
        out.push(...m.subagentTree);
      }
    }
    return out;
  }, [filteredMainMessages]);
  const anyRunning = subagentEntries.some((e) => e.status === 'running');
  // Predicate: is the pending AskUser destined for a subagent tab? If
  // yes, the pane owns the widget (and we suppress the inline render);
  // if no, the main chat keeps it (current behaviour).
  const askUserBelongsToSubagent = useMemo(() => {
    if (!pendingAskUser) return false;
    const id = pendingAskUser.agentId;
    return subagentEntries.some(
      (e) => e.subagentId === id || e.agentName === id,
    );
  }, [pendingAskUser, subagentEntries]);
  const [paneVisible, setPaneVisible] = useState(false);
  useEffect(() => {
    if (subagentEntries.length === 0) {
      setPaneVisible(false);
      return;
    }
    if (anyRunning || askUserBelongsToSubagent) {
      // Force-open the pane whenever a subagent is running or needs a
      // user answer — cancels any auto-collapse in flight.
      setPaneVisible(true);
      return;
    }
    // All done, no pending widget — schedule auto-collapse. 10 min
    // gives ample time to read the subagent's full chat without the
    // pane vanishing mid-scroll. Cancelled if a new subagent spawns.
    setPaneVisible(true);
    const t = setTimeout(() => setPaneVisible(false), 600_000);
    return () => clearTimeout(t);
  }, [anyRunning, subagentEntries.length, askUserBelongsToSubagent]);

  // Defensive idle reset: when no subagents are running for this
  // session, force the session-level agentStatus to `idle` even if a
  // late event polluted it. handleSubagentResult tries this once per
  // SubagentResult event, but a straggler ContentBlock or AgentStatus
  // event arriving after my unregister deferral could re-set the
  // session to 'thinking'/'calling_tool'. Re-asserting on the derived
  // "all done" transition is cheap and idempotent.
  useEffect(() => {
    if (!sessionId) return;
    if (subagentEntries.length === 0) return;
    if (anyRunning) return;
    const store = useServerStore.getState();
    const current = store.agentStatus[sessionId];
    if (current && current !== 'idle') {
      store.setAgentStatus((prev) => ({ ...prev, [sessionId]: 'idle' }));
      store.setAgentStatusText((prev) => ({ ...prev, [sessionId]: 'Idle' }));
    }
  }, [sessionId, anyRunning, subagentEntries.length]);

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
                const sessionMeta = useSessionStore.getState().allSessions.find(s => s.id === sessionId);
                return (
                  <>
                    <span className="font-semibold uppercase tracking-wider text-slate-500">Session</span>
                    <span className="font-mono truncate max-w-[160px]">{sessionId}</span>
                    {useUserStore.getState().userType === 'owner' && (sessionMeta?.project_name || sessionMeta?.cwd) && (() => {
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
            <div className="mt-1.5 flex flex-wrap items-center justify-between gap-2">
              <div className="flex flex-wrap items-center gap-2">
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
              <ChatDebugActions projectRoot={projectRoot} sessionId={sessionId} />
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

      <div className="relative flex-1 flex flex-col md:flex-row gap-0 min-h-0">
      <div
        ref={chatScrollRef}
        className={cn(
          'relative overflow-y-scroll px-2 py-1.5 flex flex-col gap-2 custom-scrollbar min-h-0',
          // 60/40 split on wide when the pane is shown; full width when it's not.
          showSubagentPane && paneVisible
            ? 'flex-1 md:flex-[3] md:min-w-0'
            : 'flex-1',
        )}
      >
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
        {pendingAskUser && onRespondToAskUser && !askUserBelongsToSubagent && (
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

      {showSubagentPane && paneVisible && (
        <div className="md:flex-[2] md:min-w-0 md:max-w-[40%] border-t md:border-t-0 border-slate-200 dark:border-white/10 min-h-[180px] md:min-h-0">
          <SubagentPane
            messages={filteredMainMessages}
            visible={paneVisible}
            pendingAskUser={askUserBelongsToSubagent ? pendingAskUser : null}
            onRespondToAskUser={onRespondToAskUser}
            onCancelAgentRun={onCancelAgentRun}
          />
        </div>
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

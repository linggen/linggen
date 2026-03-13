import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Sparkles, ArrowDown } from 'lucide-react';
import 'highlight.js/styles/github.css';
import { cn } from '../../lib/cn';
import { useProjectStore } from '../../stores/projectStore';
import { useUiStore } from '../../stores/uiStore';
import { useAgentStore } from '../../stores/agentStore';
import { AskUserCard } from '../AskUserCard';
import { ToolPermissionCard } from '../ToolPermissionCard';
import type {
  AgentInfo,
  AgentRunInfo,
  AgentRunContextResponse,
  ChatMessage,
  QueuedChatItem,
  SkillInfo,
  SubagentInfo,
} from '../../types';
import { normalizeAgentKey, sortMessagesByTime, contextMessageToChatMessage, mergeMessageStreams, collapseProgressMessages } from './utils/message';
import { formatRunLabel, formatTs, buildRunTimeline } from './utils/timeline';
import { getMessagePhase } from './MessagePhase';
import { AgentMessage } from './AgentMessage';
import { ChatInput } from './ChatInput';
import { SubagentDrawer } from './SubagentDrawer';
import { statusBadgeClass } from './MessageHelpers';

/** Render a single message row. */
const ChatMessageRow = React.memo<{
  msg: ChatMessage;
  msgKey: string;
  isUser: boolean;
  isExpanded: boolean;
  onToggle: () => void;
  isLastUser: boolean;
  lastUserMsgRef?: React.RefObject<HTMLDivElement | null>;
  planProps: {
    pendingPlanAgentId?: string | null;
    agentContext?: Record<string, { tokens: number; messages: number; tokenLimit?: number }>;
    onApprovePlan?: (clearContext: boolean) => void;
    onRejectPlan?: () => void;
    onEditPlan?: (text: string) => void;
    inputRef: React.RefObject<HTMLTextAreaElement | null>;
  };
}>(({ msg, msgKey, isUser, isExpanded, onToggle, isLastUser, lastUserMsgRef, planProps }) => {
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
      ref={isLastUser ? lastUserMsgRef : undefined}
      className={cn('w-full flex', isUser ? 'justify-end' : 'justify-start')}
    >
      <div className={cn(isUser ? 'max-w-[96%]' : 'max-w-full', 'text-[13px] leading-relaxed', messageClass)}>
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
  lastUserMsgInfo: { index: number; text: string } | null;
  lastUserMsgRef: React.RefObject<HTMLDivElement | null>;
  selectedAgent: string;
  pendingPlanAgentId?: string | null;
  agentContext?: Record<string, { tokens: number; messages: number; tokenLimit?: number }>;
  onApprovePlan?: (clearContext: boolean) => void;
  onRejectPlan?: () => void;
  onEditPlan?: (text: string) => void;
  inputRef: React.RefObject<HTMLTextAreaElement | null>;
}>(({ messages, expandedMessages, setExpandedMessages, verboseMode, lastUserMsgInfo, lastUserMsgRef, selectedAgent, pendingPlanAgentId, agentContext, onApprovePlan, onRejectPlan, onEditPlan, inputRef }) => {
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
        const isLastUser = isUser && lastUserMsgInfo?.index === i;
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
            isLastUser={isLastUser}
            lastUserMsgRef={lastUserMsgRef}
            planProps={planProps}
          />
        );
      })}
    </>
  );
});

/** Compact per-session model selector shown in the run bar. */
const SessionModelSelector: React.FC = () => {
  const models = useAgentStore((s) => s.models);
  const defaultModels = useAgentStore((s) => s.defaultModels);
  const sessionModel = useUiStore((s) => s.sessionModel);
  const setSessionModel = useUiStore((s) => s.setSessionModel);

  const defaultLabel = defaultModels.length > 0 ? defaultModels[0] : 'default';

  return (
    <select
      value={sessionModel ?? ''}
      onChange={(e) => setSessionModel(e.target.value || null)}
      onClick={(e) => e.stopPropagation()}
      className="ml-auto text-[10px] bg-white dark:bg-black/30 border border-slate-200 dark:border-white/10 rounded px-1.5 py-0.5 outline-none max-w-[14rem] truncate"
      title="Session model override"
    >
      <option value="">Default ({defaultLabel})</option>
      {models.map((m) => (
        <option key={m.id} value={m.id}>{m.id}</option>
      ))}
    </select>
  );
};

export const ChatPanel: React.FC<{
  chatMessages: ChatMessage[];
  queuedMessages: QueuedChatItem[];
  chatEndRef: React.RefObject<HTMLDivElement | null>;
  projectRoot?: string | null;
  selectedAgent: string;
  setSelectedAgent: (value: string) => void;
  skills: SkillInfo[];
  agents: AgentInfo[];
  mainAgents: AgentInfo[];
  subagents: SubagentInfo[];
  mainRunIds?: Record<string, string>;
  subagentRunIds?: Record<string, string>;
  runningMainRunIds?: Record<string, string>;
  runningSubagentRunIds?: Record<string, string>;
  mainRunHistory?: Record<string, AgentRunInfo[]>;
  subagentRunHistory?: Record<string, AgentRunInfo[]>;
  cancellingRunIds?: Record<string, boolean>;
  onCancelRun?: (runId: string) => void | Promise<void>;
  onSendMessage: (message: string, targetAgent?: string, images?: string[]) => void;
  activePlan?: import('../../types').Plan | null;
  pendingPlan?: import('../../types').Plan | null;
  pendingPlanAgentId?: string | null;
  agentContext?: Record<string, { tokens: number; messages: number; tokenLimit?: number }>;
  onApprovePlan?: (clearContext: boolean) => void;
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
}> = ({
  chatMessages,
  queuedMessages,
  chatEndRef,
  projectRoot,
  selectedAgent,
  setSelectedAgent,
  skills,
  agents,
  mainAgents,
  subagents,
  mainRunIds,
  subagentRunIds,
  runningMainRunIds,
  runningSubagentRunIds,
  mainRunHistory,
  subagentRunHistory,
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
}) => {
  const [openSubagentId, setOpenSubagentId] = useState<string | null>(null);
  const [selectedMainRunByAgent, setSelectedMainRunByAgent] = useState<Record<string, string>>({});
  const [selectedSubagentRunById, setSelectedSubagentRunById] = useState<Record<string, string>>({});
  const [pinnedMainRunByAgent, setPinnedMainRunByAgent] = useState<Record<string, boolean>>({});
  const [pinnedSubagentRunById, setPinnedSubagentRunById] = useState<Record<string, boolean>>({});
  const [mainMessageFilter, setMainMessageFilter] = useState('');
  const [subagentMessageFilter, setSubagentMessageFilter] = useState('');
  const [runContextById, setRunContextById] = useState<Record<string, AgentRunContextResponse>>({});
  const [loadingContextByRunId, setLoadingContextByRunId] = useState<Record<string, boolean>>({});
  const [contextErrorByRunId, setContextErrorByRunId] = useState<Record<string, string>>({});
  const [childrenByRunId, setChildrenByRunId] = useState<Record<string, AgentRunInfo[]>>({});
  const [loadingChildrenByRunId, setLoadingChildrenByRunId] = useState<Record<string, boolean>>({});
  const [childrenErrorByRunId, setChildrenErrorByRunId] = useState<Record<string, string>>({});
  const notFoundRunIds = useRef<Set<string>>(new Set());
  const prevProjectRootRef = useRef(projectRoot);

  if (projectRoot !== prevProjectRootRef.current) {
    notFoundRunIds.current.clear();
    prevProjectRootRef.current = projectRoot;
  }

  const [expandedMessages, setExpandedMessages] = useState<Set<string>>(new Set());
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const chatScrollRef = useRef<HTMLDivElement | null>(null);
  const lastUserMsgRef = useRef<HTMLDivElement | null>(null);
  const [showLastUserMsg, setShowLastUserMsg] = useState(false);
  const thinkingStartRef = useRef<number | null>(null);
  const [thinkingElapsed, setThinkingElapsed] = useState(0);

  // When chat is cleared (chatMessages becomes empty), also clear cached run
  // context so stale messages from previous runs don't keep showing.
  useEffect(() => {
    if (chatMessages.length === 0) {
      setRunContextById({});
      setContextErrorByRunId({});
      setChildrenByRunId({});
      setChildrenErrorByRunId({});
      notFoundRunIds.current.clear();
    }
  }, [chatMessages.length]);

  useEffect(() => {
    if (pendingAskUser) {
      chatEndRef?.current?.scrollIntoView({ behavior: 'auto', block: 'nearest' });
    }
  }, [pendingAskUser, chatEndRef]);

  // Auto-scroll to bottom during streaming, but only if user is near bottom.
  // "Near bottom" = within 10% of scroll height (respects manual scroll-up).
  const isNearBottomRef = useRef(true);
  const [showScrollButton, setShowScrollButton] = useState(false);
  useEffect(() => {
    const container = chatScrollRef.current;
    if (!container) return;
    const onScroll = () => {
      const { scrollTop, scrollHeight, clientHeight } = container;
      const distanceFromBottom = scrollHeight - scrollTop - clientHeight;
      const nearBottom = distanceFromBottom <= scrollHeight * 0.1;
      isNearBottomRef.current = nearBottom;
      // Only show when content is tall enough to scroll meaningfully (> 1.5x viewport)
      const contentOverflows = scrollHeight > clientHeight * 1.5;
      setShowScrollButton(!nearBottom && distanceFromBottom > 100 && contentOverflows);
    };
    container.addEventListener('scroll', onScroll, { passive: true });
    return () => container.removeEventListener('scroll', onScroll);
  }, []);

  const scrollToBottom = useCallback(() => {
    chatEndRef?.current?.scrollIntoView({ behavior: 'smooth', block: 'end' });
  }, [chatEndRef]);

  useEffect(() => {
    if (isNearBottomRef.current) {
      chatEndRef?.current?.scrollIntoView({ behavior: 'auto', block: 'nearest' });
    }
  }, [chatMessages, chatEndRef]);

  // Track agent active state and elapsed time
  const agentStatusText = useAgentStore((s) => s.agentStatusText);
  const currentStatus = agentStatus?.[selectedAgent];
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

  // Show floating banner when last user message scrolls out of view
  useEffect(() => {
    const el = lastUserMsgRef.current;
    const container = chatScrollRef.current;
    if (!el || !container) { setShowLastUserMsg(false); return; }
    const observer = new IntersectionObserver(
      ([entry]) => setShowLastUserMsg(!entry.isIntersecting),
      { root: container, threshold: 0.1 }
    );
    observer.observe(el);
    return () => observer.disconnect();
  });

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
  const selectedMainRunOptions = useMemo(
    () => mainRunHistory?.[selectedAgentKey] || [],
    [mainRunHistory, selectedAgentKey]
  );
  const selectedMainRunOverride = selectedMainRunByAgent[selectedAgentKey];
  const selectedMainPinned = !!pinnedMainRunByAgent[selectedAgentKey];
  const selectedMainRunId =
    selectedMainPinned &&
    selectedMainRunOverride &&
    selectedMainRunOptions.some((run) => run.run_id === selectedMainRunOverride)
      ? selectedMainRunOverride
      : mainRunIds?.[selectedAgentKey] || selectedMainRunOptions[0]?.run_id;
  const selectedMainRunningRunId = runningMainRunIds?.[selectedAgentKey];
  const selectedSubagentKey = selectedSubagent ? normalizeAgentKey(selectedSubagent.id) : '';
  const selectedSubagentRunOptions = useMemo(
    () => (selectedSubagent ? subagentRunHistory?.[selectedSubagentKey] || [] : []),
    [selectedSubagent, subagentRunHistory, selectedSubagentKey]
  );
  const selectedSubagentRunOverride = selectedSubagentKey
    ? selectedSubagentRunById[selectedSubagentKey]
    : undefined;
  const selectedSubagentPinned = selectedSubagentKey
    ? !!pinnedSubagentRunById[selectedSubagentKey]
    : false;
  const selectedSubagentRunId =
    selectedSubagent &&
    selectedSubagentPinned &&
    selectedSubagentRunOverride &&
    selectedSubagentRunOptions.some((run) => run.run_id === selectedSubagentRunOverride)
      ? selectedSubagentRunOverride
      : selectedSubagent
        ? subagentRunIds?.[selectedSubagentKey] || selectedSubagentRunOptions[0]?.run_id
        : undefined;
  const selectedSubagentRunningRunId = selectedSubagent
    ? runningSubagentRunIds?.[selectedSubagentKey]
    : undefined;
  const selectedMainContext = selectedMainRunId ? runContextById[selectedMainRunId] : undefined;
  const selectedSubagentContext = selectedSubagentRunId ? runContextById[selectedSubagentRunId] : undefined;
  const selectedMainContextError = selectedMainRunId ? contextErrorByRunId[selectedMainRunId] : undefined;
  const selectedSubagentContextError = selectedSubagentRunId
    ? contextErrorByRunId[selectedSubagentRunId]
    : undefined;
  const selectedMainContextLoading = selectedMainRunId
    ? !!loadingContextByRunId[selectedMainRunId]
    : false;
  const selectedSubagentContextLoading = selectedSubagentRunId
    ? !!loadingContextByRunId[selectedSubagentRunId]
    : false;
  const selectedMainChildren = useMemo(
    () => (selectedMainRunId ? childrenByRunId[selectedMainRunId] || [] : []),
    [selectedMainRunId, childrenByRunId]
  );
  const selectedSubagentChildren = useMemo(
    () => (selectedSubagentRunId ? childrenByRunId[selectedSubagentRunId] || [] : []),
    [selectedSubagentRunId, childrenByRunId]
  );
  const selectedMainChildrenLoading = selectedMainRunId
    ? !!loadingChildrenByRunId[selectedMainRunId]
    : false;
  const selectedSubagentChildrenLoading = selectedSubagentRunId
    ? !!loadingChildrenByRunId[selectedSubagentRunId]
    : false;
  const selectedMainChildrenError = selectedMainRunId
    ? childrenErrorByRunId[selectedMainRunId]
    : undefined;
  const selectedSubagentChildrenError = selectedSubagentRunId
    ? childrenErrorByRunId[selectedSubagentRunId]
    : undefined;
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
  const mainContextMessages = useMemo(
    () => (selectedMainContext?.messages || []).map(contextMessageToChatMessage).filter((m): m is ChatMessage => m !== null),
    [selectedMainContext]
  );
  const selectedSubagentContextMessages = useMemo(
    () => (selectedSubagentContext?.messages || []).map(contextMessageToChatMessage).filter((m): m is ChatMessage => m !== null),
    [selectedSubagentContext]
  );
  const displayedMainMessages = useMemo(
    () => collapseProgressMessages(mergeMessageStreams(mainContextMessages, sortMessagesByTime(visibleMessages))),
    [mainContextMessages, visibleMessages]
  );
  const displayedSubagentMessages = useMemo(
    () => mergeMessageStreams(selectedSubagentContextMessages, subagentMessages),
    [selectedSubagentContextMessages, subagentMessages]
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

  const lastUserMsgInfo = useMemo(() => {
    for (let i = filteredMainMessages.length - 1; i >= 0; i--) {
      if (filteredMainMessages[i].role === 'user') {
        return { index: i, text: filteredMainMessages[i].text };
      }
    }
    return null;
  }, [filteredMainMessages]);
  const filteredSubagentMessages = useMemo(() => {
    const q = subagentMessageFilter.trim().toLowerCase();
    if (!q) return displayedSubagentMessages;
    return displayedSubagentMessages.filter((msg) => {
      const from = normalizeAgentKey(msg.from || msg.role);
      const to = normalizeAgentKey(msg.to || '');
      return (
        msg.text.toLowerCase().includes(q) ||
        from.includes(q) ||
        to.includes(q)
      );
    });
  }, [displayedSubagentMessages, subagentMessageFilter]);
  const selectedMainTimeline = useMemo(
    () => buildRunTimeline(selectedMainContext?.run, selectedMainContext?.messages || [], selectedMainChildren),
    [selectedMainContext, selectedMainChildren]
  );
  const selectedSubagentTimeline = useMemo(
    () => buildRunTimeline(selectedSubagentContext?.run, selectedSubagentContext?.messages || [], selectedSubagentChildren),
    [selectedSubagentContext, selectedSubagentChildren]
  );

  const fetchRunContext = useCallback(
    (runId?: string, force = false) => {
      if (!runId) return;
      if (notFoundRunIds.current.has(runId)) return;
      if (loadingContextByRunId[runId]) return;
      if (!force && runContextById[runId]) return;
      setLoadingContextByRunId((prev) => ({ ...prev, [runId]: true }));
      setContextErrorByRunId((prev) => {
        const next = { ...prev };
        delete next[runId];
        return next;
      });
      void (async () => {
        try {
          const url = new URL('/api/agent-context', window.location.origin);
          url.searchParams.append('run_id', runId);
          url.searchParams.append('view', 'raw');
          if (projectRoot) url.searchParams.append('project_root', projectRoot);
          const resp = await fetch(url.toString());
          if (resp.status === 404) { notFoundRunIds.current.add(runId); return; }
          if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
          const data = (await resp.json()) as AgentRunContextResponse;
          setRunContextById((prev) => ({ ...prev, [runId]: data }));
        } catch (e) {
          const errorMessage = e instanceof Error ? e.message : String(e);
          setContextErrorByRunId((prev) => ({ ...prev, [runId]: errorMessage }));
        } finally {
          setLoadingContextByRunId((prev) => {
            const next = { ...prev };
            delete next[runId];
            return next;
          });
        }
      })();
    },
    [runContextById, loadingContextByRunId, projectRoot]
  );

  const fetchRunChildren = useCallback(
    (runId?: string, force = false) => {
      if (!runId) return;
      if (notFoundRunIds.current.has(runId)) return;
      if (loadingChildrenByRunId[runId]) return;
      if (!force && childrenByRunId[runId]) return;
      setLoadingChildrenByRunId((prev) => ({ ...prev, [runId]: true }));
      setChildrenErrorByRunId((prev) => {
        const next = { ...prev };
        delete next[runId];
        return next;
      });
      void (async () => {
        try {
          const url = new URL('/api/agent-children', window.location.origin);
          url.searchParams.append('run_id', runId);
          if (projectRoot) url.searchParams.append('project_root', projectRoot);
          const resp = await fetch(url.toString());
          if (resp.status === 404) { notFoundRunIds.current.add(runId); return; }
          if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
          const data = (await resp.json()) as AgentRunInfo[];
          setChildrenByRunId((prev) => ({ ...prev, [runId]: Array.isArray(data) ? data : [] }));
        } catch (e) {
          const errorMessage = e instanceof Error ? e.message : String(e);
          setChildrenErrorByRunId((prev) => ({ ...prev, [runId]: errorMessage }));
        } finally {
          setLoadingChildrenByRunId((prev) => {
            const next = { ...prev };
            delete next[runId];
            return next;
          });
        }
      })();
    },
    [childrenByRunId, loadingChildrenByRunId, projectRoot]
  );

  useEffect(() => {
    if (!openSubagentId) return;
    if (!subagents.some((sub) => sub.id === openSubagentId)) {
      setOpenSubagentId(null);
    }
  }, [openSubagentId, subagents]);

  useEffect(() => {
    if (!selectedMainPinned || !selectedMainRunOverride) return;
    if (selectedMainRunOptions.some((run) => run.run_id === selectedMainRunOverride)) return;
    setPinnedMainRunByAgent((prev) => {
      const next = { ...prev };
      delete next[selectedAgentKey];
      return next;
    });
    setSelectedMainRunByAgent((prev) => {
      const next = { ...prev };
      delete next[selectedAgentKey];
      return next;
    });
  }, [selectedMainPinned, selectedMainRunOverride, selectedMainRunOptions, selectedAgentKey]);

  useEffect(() => {
    if (!selectedSubagentKey || !selectedSubagentPinned || !selectedSubagentRunOverride) return;
    if (selectedSubagentRunOptions.some((run) => run.run_id === selectedSubagentRunOverride)) return;
    setPinnedSubagentRunById((prev) => {
      const next = { ...prev };
      delete next[selectedSubagentKey];
      return next;
    });
    setSelectedSubagentRunById((prev) => {
      const next = { ...prev };
      delete next[selectedSubagentKey];
      return next;
    });
  }, [selectedSubagentKey, selectedSubagentPinned, selectedSubagentRunOverride, selectedSubagentRunOptions]);

  useEffect(() => {
    fetchRunContext(selectedMainRunId);
  }, [selectedMainRunId, fetchRunContext]);

  useEffect(() => {
    fetchRunContext(selectedSubagentRunId);
  }, [selectedSubagentRunId, fetchRunContext]);

  useEffect(() => {
    fetchRunChildren(selectedMainRunId);
  }, [selectedMainRunId, fetchRunChildren]);

  useEffect(() => {
    fetchRunChildren(selectedSubagentRunId);
  }, [selectedSubagentRunId, fetchRunChildren]);

  useEffect(() => {
    if (!selectedMainRunningRunId && !selectedSubagentRunningRunId) return;
    const id = window.setInterval(() => {
      if (selectedMainRunningRunId) fetchRunContext(selectedMainRunningRunId, true);
      if (selectedSubagentRunningRunId) fetchRunContext(selectedSubagentRunningRunId, true);
      if (selectedMainRunningRunId) fetchRunChildren(selectedMainRunningRunId, true);
      if (selectedSubagentRunningRunId) fetchRunChildren(selectedSubagentRunningRunId, true);
    }, 2000);
    return () => window.clearInterval(id);
  }, [selectedMainRunningRunId, selectedSubagentRunningRunId, fetchRunContext, fetchRunChildren]);

  const prevMainRunningRef = useRef(selectedMainRunningRunId);
  const prevSubRunningRef = useRef(selectedSubagentRunningRunId);
  useEffect(() => {
    const prevMain = prevMainRunningRef.current;
    const prevSub = prevSubRunningRef.current;
    prevMainRunningRef.current = selectedMainRunningRunId;
    prevSubRunningRef.current = selectedSubagentRunningRunId;
    if (prevMain && !selectedMainRunningRunId) fetchRunContext(prevMain, true);
    if (prevSub && !selectedSubagentRunningRunId) fetchRunContext(prevSub, true);
  }, [selectedMainRunningRunId, selectedSubagentRunningRunId, fetchRunContext]);

  return (
    <section className="h-full flex flex-col bg-white dark:bg-[#0f0f0f] rounded-xl border border-slate-200 dark:border-white/5 overflow-hidden min-h-0 relative">
      <div className="px-1.5 py-1 border-b border-slate-200 dark:border-white/5 bg-slate-50/70 dark:bg-white/[0.02] space-y-1">
        {selectedMainRunId && (
          <details className="rounded-md border border-slate-200 dark:border-white/10 bg-white/80 dark:bg-black/20 px-2 py-1 text-[10px] text-slate-600 dark:text-slate-300">
            <summary className="cursor-pointer flex flex-wrap items-center gap-2">
              <span className="font-semibold uppercase tracking-wider text-slate-500">Run</span>
              <span className="font-mono truncate">{selectedMainRunId}</span>
              {selectedMainContext?.run?.status && (
                <span className={cn('px-1.5 py-0.5 rounded-full uppercase tracking-wide', statusBadgeClass(selectedMainContext.run.status))}>
                  {selectedMainContext.run.status}
                </span>
              )}
              <SessionModelSelector />
            </summary>
            <div className="mt-1.5 flex flex-wrap items-center gap-2">
              {selectedMainRunOptions.length > 1 && (
                <select
                  value={selectedMainRunId}
                  onChange={(e) => {
                    const runId = e.target.value;
                    setSelectedMainRunByAgent((prev) => ({ ...prev, [selectedAgentKey]: runId }));
                    setPinnedMainRunByAgent((prev) => ({ ...prev, [selectedAgentKey]: true }));
                  }}
                  className="text-[10px] bg-white dark:bg-black/30 border border-slate-200 dark:border-white/10 rounded px-2 py-1 outline-none min-w-[10rem]"
                  title="Select run context"
                >
                  {selectedMainRunOptions.map((run) => (
                    <option key={run.run_id} value={run.run_id}>
                      {formatRunLabel(run)}
                    </option>
                  ))}
                </select>
              )}
              <button
                onClick={() => {
                  if (selectedMainPinned) {
                    setPinnedMainRunByAgent((prev) => {
                      const next = { ...prev };
                      delete next[selectedAgentKey];
                      return next;
                    });
                    setSelectedMainRunByAgent((prev) => {
                      const next = { ...prev };
                      delete next[selectedAgentKey];
                      return next;
                    });
                  } else {
                    setSelectedMainRunByAgent((prev) => ({ ...prev, [selectedAgentKey]: selectedMainRunId }));
                    setPinnedMainRunByAgent((prev) => ({ ...prev, [selectedAgentKey]: true }));
                  }
                }}
                className={cn(
                  'px-2 py-1 rounded border text-[10px] font-semibold',
                  selectedMainPinned
                    ? 'bg-slate-100 text-slate-600 border-slate-300'
                    : 'bg-blue-50 text-blue-600 border-blue-200'
                )}
                title={selectedMainPinned ? 'Unpin run selection' : 'Pin this run selection'}
              >
                {selectedMainPinned ? 'Unpin' : 'Pin'}
              </button>
              {selectedMainRunningRunId && onCancelRun && (
                <button
                  onClick={() => onCancelRun(selectedMainRunningRunId)}
                  disabled={!!cancellingRunIds?.[selectedMainRunningRunId]}
                  className={cn(
                    'px-2 py-1 rounded border text-[10px] font-semibold transition-colors',
                    cancellingRunIds?.[selectedMainRunningRunId]
                      ? 'bg-slate-100 text-slate-400 border-slate-200 cursor-not-allowed'
                      : 'bg-red-50 text-red-600 border-red-200 hover:bg-red-100'
                  )}
                  title={selectedMainRunningRunId}
                >
                  {cancellingRunIds?.[selectedMainRunningRunId] ? 'Cancelling...' : 'Cancel Run'}
                </button>
              )}
              {selectedMainContextLoading && <span className="text-blue-500">Loading context...</span>}
              {selectedMainContextError && <span className="text-red-500">Context error: {selectedMainContextError}</span>}
              {selectedMainChildrenLoading && <span className="text-blue-500">Loading child runs...</span>}
              {selectedMainChildrenError && <span className="text-red-500">Children error: {selectedMainChildrenError}</span>}
            </div>
            {selectedMainContext?.summary && (
              <div className="mt-1 text-slate-500 dark:text-slate-400">
                msgs {selectedMainContext.summary.message_count} • user {selectedMainContext.summary.user_messages} • agent {selectedMainContext.summary.agent_messages} • system {selectedMainContext.summary.system_messages}
              </div>
            )}
          </details>
        )}

        {subagents.length > 0 && (
          <details className="rounded-md border border-slate-200 dark:border-white/10 bg-white/80 dark:bg-black/20 px-2 py-1 text-[10px]">
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
                    'px-2 py-1 rounded-md text-[10px] border transition-colors flex items-center gap-1',
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
        {showLastUserMsg && lastUserMsgInfo && (
          <div
            className="sticky top-0 z-20 mx-1 mb-1 px-3 py-2 rounded-md bg-slate-100/95 dark:bg-white/10 backdrop-blur text-[12px] text-slate-700 dark:text-slate-200 border border-slate-200/60 dark:border-white/10 cursor-pointer"
            onClick={() => lastUserMsgRef.current?.scrollIntoView({ behavior: 'smooth', block: 'center' })}
            title={lastUserMsgInfo.text}
          >
            <p className="line-clamp-4 whitespace-pre-wrap break-words">
              <span className="font-medium text-slate-500 dark:text-slate-400 mr-1.5">You:</span>
              {lastUserMsgInfo.text}
            </p>
          </div>
        )}
        <div className="flex items-center justify-between gap-2 mb-1">
          {selectedMainTimeline.length > 0 ? (
            <details className="text-[10px] text-slate-500">
              <summary className="cursor-pointer">Timeline ({selectedMainTimeline.length})</summary>
              <div className="mt-1 space-y-1 max-h-28 overflow-auto custom-scrollbar pr-2">
                {selectedMainTimeline.map((evt, idx) => (
                  <div key={`${evt.ts}-${evt.label}-${idx}`} className="text-[10px] text-slate-500 dark:text-slate-400">
                    {formatTs(evt.ts)} • {evt.label}
                    {evt.detail ? ` • ${evt.detail}` : ''}
                  </div>
                ))}
              </div>
            </details>
          ) : (
            <div />
          )}
          <input
            value={mainMessageFilter}
            onChange={(e) => setMainMessageFilter(e.target.value)}
            placeholder="Filter messages"
            className="w-52 text-[11px] bg-white dark:bg-black/30 border border-slate-200 dark:border-white/10 rounded px-2 py-1 outline-none"
          />
        </div>
        <ChatMessageList
          messages={historicalMessages}
          expandedMessages={expandedMessages}
          setExpandedMessages={setExpandedMessages}
          verboseMode={verboseMode}
          lastUserMsgInfo={streamingMessage ? null : lastUserMsgInfo}
          lastUserMsgRef={lastUserMsgRef}
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
            isLastUser={false}
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
          <div className="flex items-center gap-1.5 text-[12px] text-slate-500 dark:text-slate-400 font-medium animate-pulse">
            <span className="text-blue-500">✶</span>
            <span>
              {agentStatusText?.[selectedAgent] || (currentStatus === 'model_loading' ? 'Loading model' : spinnerVerb || 'Thinking')}…
              {(thinkingElapsed > 0 || (agentContext?.[selectedAgent]?.tokens ?? 0) > 0) && (
                <span className="font-normal text-slate-400 dark:text-slate-500 ml-1">
                  ({[
                    thinkingElapsed >= 60
                      ? `${Math.floor(thinkingElapsed / 60)}m ${thinkingElapsed % 60}s`
                      : thinkingElapsed > 0 ? `${thinkingElapsed}s` : '',
                    (tokensPerSec ?? 0) > 0
                      ? `${tokensPerSec!.toFixed(1)} tok/s`
                      : '',
                    (agentContext?.[selectedAgent]?.tokens ?? 0) > 0
                      ? (() => {
                          const t = agentContext?.[selectedAgent]?.tokens ?? 0;
                          const lim = agentContext?.[selectedAgent]?.tokenLimit;
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
          <div className="flex items-center gap-1.5 text-[12px] text-slate-400 dark:text-slate-500 italic">
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
      />

      {selectedSubagent && (
        <SubagentDrawer
          selectedSubagent={selectedSubagent}
          selectedSubagentKey={selectedSubagentKey}
          selectedSubagentRunId={selectedSubagentRunId}
          selectedSubagentRunningRunId={selectedSubagentRunningRunId}
          selectedSubagentRunOptions={selectedSubagentRunOptions}
          selectedSubagentPinned={selectedSubagentPinned}
          selectedSubagentContext={selectedSubagentContext}
          selectedSubagentContextLoading={selectedSubagentContextLoading}
          selectedSubagentContextError={selectedSubagentContextError}
          selectedSubagentChildrenLoading={selectedSubagentChildrenLoading}
          selectedSubagentChildrenError={selectedSubagentChildrenError}
          selectedSubagentTimeline={selectedSubagentTimeline}
          filteredSubagentMessages={filteredSubagentMessages}
          subagentMessageFilter={subagentMessageFilter}
          setSubagentMessageFilter={setSubagentMessageFilter}
          cancellingRunIds={cancellingRunIds}
          onCancelRun={onCancelRun}
          onClose={() => setOpenSubagentId(null)}
          setSelectedSubagentRunById={setSelectedSubagentRunById}
          setPinnedSubagentRunById={setPinnedSubagentRunById}
        />
      )}
    </section>
  );
};

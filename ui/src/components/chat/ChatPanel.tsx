import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Send, Square, X, Sparkles, FolderOpen, FileText } from 'lucide-react';
import 'highlight.js/styles/github.css';
import { cn } from '../../lib/cn';
import { AskUserCard } from '../AskUserCard';
import { ToolPermissionCard } from '../ToolPermissionCard';
import type {
  AgentInfo,
  AgentRunInfo,
  AgentRunContextResponse,
  ChatMessage,
  FileEntry,
  QueuedChatItem,
  SkillInfo,
  SubagentInfo,
} from '../../types';
import { normalizeAgentKey, sortMessagesByTime, contextMessageToChatMessage, mergeMessageStreams, collapseProgressMessages } from './utils/message';
import { formatRunLabel, formatTs, buildRunTimeline } from './utils/timeline';
import { getMessagePhase } from './MessagePhase';
import { visibleMessageText } from './MessageHelpers';
import { MarkdownContent } from './MarkdownContent';
import { AgentMessage } from './AgentMessage';

const statusBadgeClass = (status?: string) => {
  if (status === 'working') return 'bg-green-500/15 text-green-600 dark:text-green-300';
  if (status === 'thinking') return 'bg-blue-500/15 text-blue-600 dark:text-blue-300';
  if (status === 'calling_tool') return 'bg-amber-500/15 text-amber-700 dark:text-amber-300';
  if (status === 'model_loading') return 'bg-indigo-500/15 text-indigo-700 dark:text-indigo-300';
  return 'bg-slate-500/15 text-slate-600 dark:text-slate-300';
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
}) => {
  const [chatInput, setChatInput] = useState('');
  const [pendingImages, setPendingImages] = useState<string[]>([]);
  const [showSkillDropdown, setShowSkillDropdown] = useState(false);
  const [skillFilter, setSkillFilter] = useState('');
  const [showAgentDropdown, setShowAgentDropdown] = useState(false);
  const [agentFilter, setAgentFilter] = useState('');
  const [showFileDropdown, setShowFileDropdown] = useState(false);
  const [fileFilter, setFileFilter] = useState('');
  const [fileBrowsePath, setFileBrowsePath] = useState('');
  const [fileEntries, setFileEntries] = useState<FileEntry[]>([]);
  const [fileEntriesLoading, setFileEntriesLoading] = useState(false);
  const [fileSearchMode, setFileSearchMode] = useState(false);
  const searchTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [selectedSuggestionIndex, setSelectedSuggestionIndex] = useState(0);
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

  // Track agent active state and elapsed time
  const currentStatus = agentStatus?.[selectedAgent];
  const isAgentActive = !!currentStatus && currentStatus !== 'idle';
  const isThinking = currentStatus === 'thinking' || currentStatus === 'model_loading';
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

  const visibleMessages = useMemo(() => {
    const selected = normalizeAgentKey(selectedAgent);
    return chatMessages.filter((msg) => {
      const from = normalizeAgentKey(msg.from || msg.role);
      const to = normalizeAgentKey(msg.to || '');
      if (msg.role === 'user') {
        return !to || to === selected;
      }
      if (from === selected || to === selected) return true;
      if (from === 'user') return to === selected;
      return false;
    });
  }, [chatMessages, selectedAgent]);

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
    () => collapseProgressMessages(mergeMessageStreams(mainContextMessages, visibleMessages)),
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

  const resizeInput = () => {
    if (!inputRef.current) return;
    inputRef.current.style.height = '0px';
    const next = Math.min(inputRef.current.scrollHeight, 220);
    inputRef.current.style.height = `${next}px`;
  };

  useEffect(() => {
    resizeInput();
  }, [chatInput]);

  const send = () => {
    if (!chatInput.trim() && pendingImages.length === 0) return;
    const userMessage = chatInput.trim();
    const imagesToSend = pendingImages.length > 0 ? [...pendingImages] : undefined;
    setChatInput('');
    setPendingImages([]);
    setShowSkillDropdown(false);
    setShowAgentDropdown(false);
    setShowFileDropdown(false);
    setFileFilter('');
    setFileBrowsePath('');
    setFileEntries([]);

    const mentionMatch = userMessage.trim().match(/^@@([a-zA-Z0-9_-]+)\b/);
    let mentionAgent: string | undefined;
    if (mentionMatch?.[1]) {
      const mentioned = normalizeAgentKey(mentionMatch[1]);
      if (mainAgentIds.includes(mentioned)) {
        mentionAgent = mentioned;
        setSelectedAgent(mentioned);
      }
    }

    const targetAgent = mentionAgent || selectedAgent;
    onSendMessage(userMessage, targetAgent, imagesToSend);
    window.setTimeout(resizeInput, 0);
  };

  const readFileAsBase64 = (file: File): Promise<string> => {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => {
        const result = reader.result as string;
        const base64 = result.split(',')[1] || result;
        resolve(base64);
      };
      reader.onerror = reject;
      reader.readAsDataURL(file);
    });
  };

  const handlePaste = async (e: React.ClipboardEvent) => {
    const items = e.clipboardData?.items;
    if (!items) return;
    for (const item of Array.from(items)) {
      if (item.type.startsWith('image/')) {
        e.preventDefault();
        const file = item.getAsFile();
        if (file) {
          const base64 = await readFileAsBase64(file);
          setPendingImages(prev => [...prev, base64]);
        }
      }
    }
  };

  const handleDrop = async (e: React.DragEvent) => {
    const files = e.dataTransfer?.files;
    if (!files) return;
    for (const file of Array.from(files)) {
      if (file.type.startsWith('image/')) {
        e.preventDefault();
        const base64 = await readFileAsBase64(file);
        setPendingImages(prev => [...prev, base64]);
      }
    }
  };

  const handleDragOver = (e: React.DragEvent) => {
    if (e.dataTransfer?.types?.includes('Files')) {
      e.preventDefault();
    }
  };

  const fetchFileEntries = useCallback(async (browsePath: string) => {
    if (!projectRoot) return;
    setFileEntriesLoading(true);
    try {
      const url = `/api/files?project_root=${encodeURIComponent(projectRoot)}&path=${encodeURIComponent(browsePath)}`;
      const resp = await fetch(url);
      if (resp.ok) {
        const entries: FileEntry[] = await resp.json();
        entries.sort((a, b) => (a.isDir !== b.isDir ? (a.isDir ? -1 : 1) : a.name.localeCompare(b.name)));
        setFileEntries(entries);
      }
    } catch {
      setFileEntries([]);
    } finally {
      setFileEntriesLoading(false);
    }
  }, [projectRoot]);

  const searchFiles = useCallback((query: string) => {
    if (!projectRoot) return;
    if (searchTimeoutRef.current) clearTimeout(searchTimeoutRef.current);
    if (!query) {
      setFileEntries([]);
      return;
    }
    searchTimeoutRef.current = setTimeout(async () => {
      setFileEntriesLoading(true);
      try {
        const url = `/api/files/search?project_root=${encodeURIComponent(projectRoot)}&query=${encodeURIComponent(query)}`;
        const resp = await fetch(url);
        if (resp.ok) setFileEntries(await resp.json());
      } catch {
        setFileEntries([]);
      } finally {
        setFileEntriesLoading(false);
      }
    }, 150);
  }, [projectRoot]);

  const filteredFileEntries = useMemo(() => {
    // In search mode, entries are already filtered by the backend
    if (fileSearchMode) return fileEntries;
    let entries = fileEntries;
    if (fileFilter) {
      entries = entries.filter((e) => e.name.toLowerCase().includes(fileFilter.toLowerCase()));
    }
    // Hide dotfiles unless filter starts with "."
    if (!fileFilter.startsWith('.')) {
      entries = entries.filter((e) => !e.name.startsWith('.'));
    }
    return entries;
  }, [fileEntries, fileFilter, fileSearchMode]);

  const buildSkillSuggestions = () => {
    const suggestions: {
      key: string;
      label: string;
      description?: string;
      apply: () => void;
    }[] = [];

    const beforeSlash = chatInput.substring(0, chatInput.lastIndexOf('/'));

    // Built-in commands (aligned with TUI)
    const builtinCommands: [string, string][] = [
      ['/help', 'Show available commands'],
      ['/clear', 'Clear chat context'],
      ['/status', 'Show project status'],
      ['/model', 'Switch default model'],
      ['/plan', 'Ask agent to create a plan'],
      ['/image', 'Attach an image file'],
    ];

    for (const [cmd, desc] of builtinCommands) {
      const name = cmd.slice(1); // strip leading /
      if (skillFilter === '' || name.includes(skillFilter) || desc.toLowerCase().includes(skillFilter)) {
        suggestions.push({
          key: `cmd-${name}`,
          label: cmd,
          description: desc,
          apply: () => {
            setChatInput(`${beforeSlash}${cmd} `);
            setShowSkillDropdown(false);
          },
        });
      }
    }

    // Skills from API
    skills
      .filter(
        (skill) =>
          skill.name.toLowerCase().includes(skillFilter) ||
          skill.description.toLowerCase().includes(skillFilter)
      )
      .forEach((skill) => {
        suggestions.push({
          key: `skill-${skill.name}`,
          label: `/${skill.name}`,
          description: skill.description,
          apply: () => {
            setChatInput(`${beforeSlash}/${skill.name} `);
            setShowSkillDropdown(false);
          },
        });
      });

    return suggestions;
  };

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
            className="sticky top-0 z-20 mx-1 mb-1 px-3 py-1.5 rounded-md bg-slate-100/95 dark:bg-white/10 backdrop-blur text-[12px] text-slate-700 dark:text-slate-200 border border-slate-200/60 dark:border-white/10 truncate cursor-pointer"
            onClick={() => lastUserMsgRef.current?.scrollIntoView({ behavior: 'smooth', block: 'center' })}
            title={lastUserMsgInfo.text}
          >
            <span className="font-medium text-slate-500 dark:text-slate-400 mr-1.5">You:</span>
            {lastUserMsgInfo.text.length > 120 ? lastUserMsgInfo.text.slice(0, 120) + '…' : lastUserMsgInfo.text}
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
        {filteredMainMessages.length === 0 && (
          <div className="self-center mt-12 max-w-md text-center">
            <div className="text-sm font-semibold text-slate-600 dark:text-slate-300">
              No messages for {selectedAgent}
            </div>
            <div className="mt-2 text-xs text-slate-500">
              Send a message to this main agent or switch tabs.
            </div>
          </div>
        )}
        {filteredMainMessages.map((msg, i) => {
          const key = `${msg.timestamp}-${i}-${msg.from || msg.role}-${msg.text.slice(0, 24)}`;
          const isUser = msg.role === 'user';
          const phase = isUser ? undefined : getMessagePhase(msg);
          const messageClass = isUser
            ? 'bg-slate-100 dark:bg-white/10 text-slate-900 dark:text-slate-100 rounded-md px-2.5 py-1.5'
            : phase === 'thinking'
              ? ''
              : msg.isThinking && !msg.isGenerating
                ? 'text-slate-500 dark:text-slate-400 italic opacity-60'
                : 'text-slate-800 dark:text-slate-200';
          const isExpanded = verboseMode || expandedMessages.has(key);
          const toggleExpand = () => {
            setExpandedMessages((prev) => {
              const next = new Set(prev);
              if (next.has(key)) next.delete(key);
              else next.add(key);
              return next;
            });
          };
          const isLastUser = isUser && lastUserMsgInfo?.index === i;
          return (
            <div
              key={key}
              ref={isLastUser ? lastUserMsgRef : undefined}
              className={cn('w-full flex', isUser ? 'justify-end' : 'justify-start')}
            >
              <div
                className={cn(
                  isUser ? 'max-w-[96%]' : 'max-w-full', 'text-[13px] leading-relaxed',
                  messageClass
                )}
              >
                {isUser ? (
                  <>
                    {msg.text}
                    {msg.images && msg.images.length > 0 && (
                      <div className="flex gap-1.5 mt-1.5 flex-wrap">
                        {msg.images.map((img, imgIdx) => (
                          <img
                            key={imgIdx}
                            src={`data:image/png;base64,${img}`}
                            alt={`Image ${imgIdx + 1}`}
                            className="max-w-[200px] max-h-[200px] rounded-md border border-slate-200 dark:border-white/10 object-contain"
                          />
                        ))}
                      </div>
                    )}
                  </>
                ) : (
                  <AgentMessage
                    msg={msg}
                    isExpanded={isExpanded}
                    onToggle={toggleExpand}
                    planProps={{
                      pendingPlanAgentId,
                      agentContext,
                      onApprovePlan,
                      onRejectPlan,
                      onEditPlan,
                      inputRef,
                    }}
                  />
                )}
              </div>
            </div>
          );
        })}
        {pendingAskUser && onRespondToAskUser && (
          <div className="px-3 py-2">
            {pendingAskUser.questions[0]?.header === 'Permission'
              ? <ToolPermissionCard pending={pendingAskUser} onRespond={onRespondToAskUser} />
              : <AskUserCard pending={pendingAskUser} onRespond={onRespondToAskUser} />}
          </div>
        )}
        <div ref={chatEndRef} />
      </div>

      {/* Status spinner — always visible when active or just completed */}
      {isAgentActive ? (
        <div className="px-3 py-1.5">
          <div className="flex items-center gap-1.5 text-[12px] text-slate-500 dark:text-slate-400 font-medium animate-pulse">
            <span className="text-blue-500">✶</span>
            <span>
              {currentStatus === 'model_loading' ? 'Loading model' : spinnerVerb || 'Thinking'}…
              {(thinkingElapsed > 0 || (agentContext?.[selectedAgent]?.tokens ?? 0) > 0) && (
                <span className="font-normal text-slate-400 dark:text-slate-500 ml-1">
                  ({[
                    thinkingElapsed >= 60
                      ? `${Math.floor(thinkingElapsed / 60)}m ${thinkingElapsed % 60}s`
                      : thinkingElapsed > 0 ? `${thinkingElapsed}s` : '',
                    (agentContext?.[selectedAgent]?.tokens ?? 0) > 0
                      ? `↑ ${((agentContext?.[selectedAgent]?.tokens ?? 0) / 1000).toFixed(1)}k tokens`
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
      <div className="sticky bottom-0 z-10 p-2 border-t border-slate-200 dark:border-white/5 space-y-2 bg-slate-50 dark:bg-white/[0.02]">
        {visibleQueued.length > 0 && (
          <div className="px-2 py-1.5 text-[11px] rounded-md border border-amber-300/40 bg-amber-50/80 dark:bg-amber-500/10 dark:border-amber-500/20">
            <div className="flex items-center gap-1.5 text-amber-600 dark:text-amber-400 font-medium select-none">
              <span className="w-1.5 h-1.5 rounded-full bg-amber-500 animate-pulse" />
              {visibleQueued.length} message{visibleQueued.length > 1 ? 's' : ''} queued — agent is busy
            </div>
            <div className="mt-1 space-y-0.5 text-amber-700 dark:text-amber-300/80">
              {visibleQueued.map((item) => (
                <div key={item.id} className="truncate pl-3">
                  {item.preview}
                </div>
              ))}
            </div>
          </div>
        )}
        {pendingImages.length > 0 && (
          <div className="flex gap-1.5 px-2 py-1.5 flex-wrap">
            {pendingImages.map((img, idx) => (
              <div key={idx} className="relative group">
                <img
                  src={`data:image/png;base64,${img}`}
                  alt={`Pending ${idx + 1}`}
                  className="w-16 h-16 object-cover rounded-md border border-slate-200 dark:border-white/10"
                />
                <button
                  onClick={() => setPendingImages(prev => prev.filter((_, i) => i !== idx))}
                  className="absolute -top-1.5 -right-1.5 w-5 h-5 rounded-full bg-red-500 text-white flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity"
                  title="Remove image"
                >
                  <X size={10} />
                </button>
              </div>
            ))}
          </div>
        )}
        <div className="flex gap-2 bg-white dark:bg-black/20 p-1.5 rounded-xl border border-slate-300/80 dark:border-white/10 relative items-end">
          {showSkillDropdown && (
            <div className="absolute bottom-full left-0 right-0 mb-2 bg-white dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-lg shadow-xl max-h-52 overflow-y-auto z-[70]">
              <div className="px-3 py-2 text-[10px] text-slate-500 border-b border-slate-200 dark:border-white/10">
                Commands &amp; Skills • Type to filter
              </div>
              {(() => {
                const suggestions = buildSkillSuggestions();
                return suggestions.map((item, idx) => (
                  <button
                    key={item.key}
                    onClick={item.apply}
                    className={cn(
                      'w-full px-3 py-2 text-left text-xs border-b border-slate-200 dark:border-white/5 last:border-none',
                      idx === selectedSuggestionIndex
                        ? 'bg-blue-500/10 text-blue-600'
                        : 'hover:bg-slate-100 dark:hover:bg-white/5'
                    )}
                  >
                    <div className={cn('font-bold', item.key.startsWith('cmd-') ? 'text-amber-500' : 'text-blue-500')}>{item.label}</div>
                    {item.description && <div className="text-slate-500 text-[10px]">{item.description}</div>}
                  </button>
                ));
              })()}
              {buildSkillSuggestions().length === 0 && (
                <div className="p-3 text-[10px] text-slate-500 italic">No matching commands or skills</div>
              )}
            </div>
          )}
          {showAgentDropdown && (
            <div className="absolute bottom-full left-0 right-0 mb-2 bg-white dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-lg shadow-xl max-h-48 overflow-y-auto z-[70]">
              {agents
                .filter((agent) => mainAgentIds.includes(normalizeAgentKey(agent.name)))
                .filter((agent) => agent.name.toLowerCase().includes(agentFilter))
                .map((agent) => (
                  <button
                    key={agent.name}
                    onClick={() => {
                      const doubleAtIdx = chatInput.lastIndexOf('@@');
                      const beforeAt = doubleAtIdx >= 0 ? chatInput.substring(0, doubleAtIdx) : chatInput;
                      const label = agent.name.charAt(0).toUpperCase() + agent.name.slice(1);
                      setChatInput(`${beforeAt}@@${label} `);
                      setShowAgentDropdown(false);
                      setSelectedAgent(agent.name.toLowerCase());
                    }}
                    className="w-full px-3 py-2 text-left hover:bg-slate-100 dark:hover:bg-white/5 text-xs border-b border-slate-200 dark:border-white/5 last:border-none"
                  >
                    <div className="font-bold text-purple-500">@@{agent.name.charAt(0).toUpperCase() + agent.name.slice(1)}</div>
                    <div className="text-slate-500 text-[10px]">{agent.description}</div>
                  </button>
                ))}
            </div>
          )}
          {showFileDropdown && (
            <div className="absolute bottom-full left-0 right-0 mb-2 bg-white dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-lg shadow-xl max-h-56 overflow-y-auto z-[70]">
              {!fileSearchMode && fileBrowsePath && (
                <div className="px-3 py-1.5 text-[10px] text-slate-500 dark:text-slate-400 border-b border-slate-200 dark:border-white/5 font-mono truncate">
                  {fileBrowsePath}
                </div>
              )}
              {fileEntriesLoading ? (
                <div className="p-3 text-[10px] text-slate-500 italic">Loading...</div>
              ) : filteredFileEntries.length === 0 ? (
                <div className="p-3 text-[10px] text-slate-500 italic">No matching files</div>
              ) : (
                filteredFileEntries.map((entry) => (
                  <button
                    key={entry.path}
                    onClick={() => {
                      const lastAt = chatInput.lastIndexOf('@');
                      const beforeAt = chatInput.substring(0, lastAt);
                      if (entry.isDir) {
                        // Navigate into directory
                        const newPath = fileSearchMode ? entry.path + '/' : fileBrowsePath + entry.name + '/';
                        setChatInput(`${beforeAt}@${newPath}`);
                        setFileBrowsePath(newPath);
                        setFileFilter('');
                        setFileSearchMode(false);
                        setSelectedSuggestionIndex(0);
                        fetchFileEntries(newPath);
                      } else {
                        // Complete file mention
                        const fullPath = fileSearchMode ? entry.path : fileBrowsePath + entry.name;
                        setChatInput(`${beforeAt}@${fullPath} `);
                        setShowFileDropdown(false);
                        setFileFilter('');
                        setFileBrowsePath('');
                        setFileSearchMode(false);
                      }
                    }}
                    className={cn(
                      'w-full px-3 py-1.5 text-left hover:bg-slate-100 dark:hover:bg-white/5 text-xs border-b border-slate-200 dark:border-white/5 last:border-none flex items-center gap-2',
                      filteredFileEntries.indexOf(entry) === selectedSuggestionIndex && 'bg-blue-500/10'
                    )}
                  >
                    {entry.isDir ? (
                      <FolderOpen size={13} className="text-amber-500 shrink-0" />
                    ) : (
                      <FileText size={13} className="text-slate-400 shrink-0" />
                    )}
                    {fileSearchMode ? (
                      <span className="font-mono truncate">{entry.path}</span>
                    ) : (
                      <span className={entry.isDir ? 'font-semibold' : ''}>{entry.name}</span>
                    )}
                  </button>
                ))
              )}
            </div>
          )}
          <textarea
            ref={inputRef}
            value={chatInput}
            onPaste={handlePaste}
            onDrop={handleDrop}
            onDragOver={handleDragOver}
            onChange={(e) => {
              const val = e.target.value;
              setChatInput(val);

              // Skill dropdown: `/` trigger
              if (val.includes('/') && !val.includes(' ', val.lastIndexOf('/'))) {
                const lastSlash = val.lastIndexOf('/');
                // Only trigger skill dropdown if the `/` is not part of a file path (i.e. after `@`)
                const atIdx = val.lastIndexOf('@');
                if (atIdx < 0 || lastSlash < atIdx) {
                  setSkillFilter(val.substring(lastSlash + 1).toLowerCase());
                  setShowSkillDropdown(true);
                  setShowAgentDropdown(false);
                  setShowFileDropdown(false);
                  setSelectedSuggestionIndex(0);
                  return;
                }
              }

              // Find last `@` not preceded by a space-after check
              const lastAt = val.lastIndexOf('@');
              if (lastAt >= 0 && !val.includes(' ', lastAt)) {
                // Check for `@@` (agent mention)
                if (lastAt > 0 && val[lastAt - 1] === '@') {
                  const afterDoubleAt = val.substring(lastAt + 1);
                  // Only show agent dropdown if no space after @@
                  if (!afterDoubleAt.includes(' ')) {
                    setAgentFilter(afterDoubleAt.toLowerCase());
                    setShowAgentDropdown(true);
                    setShowSkillDropdown(false);
                    setShowFileDropdown(false);
                    setSelectedSuggestionIndex(0);
                    return;
                  }
                }

                // Single `@` — file mention
                const afterAt = val.substring(lastAt + 1);
                // Don't trigger file dropdown if next char is also `@`
                if (!afterAt.startsWith('@') && !afterAt.includes(' ')) {
                  const pathText = afterAt;
                  const lastSlashInPath = pathText.lastIndexOf('/');

                  setShowFileDropdown(true);
                  setShowAgentDropdown(false);
                  setShowSkillDropdown(false);
                  setSelectedSuggestionIndex(0);

                  if (lastSlashInPath >= 0) {
                    // Has `/` → directory browse mode
                    const browsePath = pathText.substring(0, lastSlashInPath + 1);
                    const filterText = pathText.substring(lastSlashInPath + 1);
                    setFileFilter(filterText);
                    setFileSearchMode(false);
                    if (browsePath !== fileBrowsePath || fileEntries.length === 0) {
                      setFileBrowsePath(browsePath);
                      fetchFileEntries(browsePath);
                    }
                  } else {
                    // No `/` → recursive search mode
                    setFileFilter(pathText);
                    setFileSearchMode(true);
                    setFileBrowsePath('');
                    searchFiles(pathText);
                  }
                  return;
                }
              }

              // No dropdown
              setShowSkillDropdown(false);
              setShowAgentDropdown(false);
              setShowFileDropdown(false);
            }}
            onKeyDown={(e) => {
              // Skill dropdown keyboard nav
              const suggestions = showSkillDropdown ? buildSkillSuggestions() : [];
              if (showSkillDropdown && suggestions.length > 0) {
                if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
                  e.preventDefault();
                  const delta = e.key === 'ArrowDown' ? 1 : -1;
                  setSelectedSuggestionIndex((prev) => (prev + delta + suggestions.length) % suggestions.length);
                  return;
                }
                if (e.key === 'Enter') {
                  // If the input exactly matches a complete built-in command, send it directly
                  const exactCommands = ['/help', '/clear', '/status', '/model'];
                  if (exactCommands.includes(chatInput.trim())) {
                    e.preventDefault();
                    setShowSkillDropdown(false);
                    send();
                    return;
                  }
                  e.preventDefault();
                  suggestions[selectedSuggestionIndex]?.apply();
                  return;
                }
              }

              // File dropdown keyboard nav
              if (showFileDropdown && filteredFileEntries.length > 0) {
                if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
                  e.preventDefault();
                  const delta = e.key === 'ArrowDown' ? 1 : -1;
                  setSelectedSuggestionIndex((prev) => (prev + delta + filteredFileEntries.length) % filteredFileEntries.length);
                  return;
                }
                if (e.key === 'Enter' || (e.key === 'Tab' && filteredFileEntries[selectedSuggestionIndex]?.isDir)) {
                  e.preventDefault();
                  const entry = filteredFileEntries[selectedSuggestionIndex];
                  if (!entry) return;
                  const lastAt = chatInput.lastIndexOf('@');
                  const beforeAt = chatInput.substring(0, lastAt);
                  if (entry.isDir) {
                    const newPath = fileSearchMode ? entry.path + '/' : fileBrowsePath + entry.name + '/';
                    setChatInput(`${beforeAt}@${newPath}`);
                    setFileBrowsePath(newPath);
                    setFileFilter('');
                    setFileSearchMode(false);
                    setSelectedSuggestionIndex(0);
                    fetchFileEntries(newPath);
                  } else {
                    const fullPath = fileSearchMode ? entry.path : fileBrowsePath + entry.name;
                    setChatInput(`${beforeAt}@${fullPath} `);
                    setShowFileDropdown(false);
                    setFileFilter('');
                    setFileBrowsePath('');
                    setFileSearchMode(false);
                  }
                  return;
                }
              }

              // Agent dropdown keyboard nav
              if (showAgentDropdown) {
                const filteredAgents = agents
                  .filter((a) => mainAgentIds.includes(normalizeAgentKey(a.name)))
                  .filter((a) => a.name.toLowerCase().includes(agentFilter));
                if (filteredAgents.length > 0) {
                  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
                    e.preventDefault();
                    const delta = e.key === 'ArrowDown' ? 1 : -1;
                    setSelectedSuggestionIndex((prev) => (prev + delta + filteredAgents.length) % filteredAgents.length);
                    return;
                  }
                  if (e.key === 'Enter') {
                    e.preventDefault();
                    const agent = filteredAgents[selectedSuggestionIndex];
                    if (!agent) return;
                    const doubleAtIdx = chatInput.lastIndexOf('@@');
                    const beforeAt = doubleAtIdx >= 0 ? chatInput.substring(0, doubleAtIdx) : chatInput;
                    const label = agent.name.charAt(0).toUpperCase() + agent.name.slice(1);
                    setChatInput(`${beforeAt}@@${label} `);
                    setShowAgentDropdown(false);
                    setSelectedAgent(agent.name.toLowerCase());
                    return;
                  }
                }
              }

              // Send on Enter (only when no dropdown open)
              if (e.key === 'Enter' && !e.shiftKey && !showSkillDropdown && !showAgentDropdown && !showFileDropdown) {
                e.preventDefault();
                send();
              }
              if (e.key === 'Escape') {
                setShowSkillDropdown(false);
                setShowAgentDropdown(false);
                setShowFileDropdown(false);
              }
            }}
            placeholder="Message... (/ for skills, @ for files, @@ for agents, Shift+Enter for newline)"
            rows={1}
            className="flex-1 bg-transparent border-none px-1.5 py-1.5 text-[13px] outline-none resize-none min-h-[34px] max-h-[200px] leading-5"
          />
          {isRunning && selectedMainRunningRunId && onCancelAgentRun && (
            <button
              onClick={() => onCancelAgentRun(selectedMainRunningRunId)}
              className="w-8 h-8 rounded-lg bg-red-500 text-white flex items-center justify-center hover:bg-red-600 transition-colors"
              title="Stop agent"
            >
              <Square size={12} fill="currentColor" />
            </button>
          )}
          <button
            onClick={send}
            className="w-8 h-8 rounded-lg bg-blue-600 text-white flex items-center justify-center hover:bg-blue-500 transition-colors"
            title="Send"
          >
            <Send size={14} />
          </button>
        </div>
      </div>

      {selectedSubagent && (
        <div className="absolute inset-y-0 right-0 w-[min(26rem,95%)] bg-white dark:bg-[#0f0f0f] border-l border-slate-200 dark:border-white/10 shadow-2xl z-[65] flex flex-col">
          <div className="px-4 py-3 border-b border-slate-200 dark:border-white/10 flex items-start justify-between gap-3">
            <div>
              <div className="text-xs font-bold uppercase tracking-wider text-slate-500">Subagent Context</div>
              <div className="mt-1 text-sm font-semibold text-slate-900 dark:text-slate-100">{selectedSubagent.id}</div>
              <div className="mt-1 text-[10px] text-slate-500 dark:text-slate-400">
                {selectedSubagent.folder}/{selectedSubagent.file}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <span className={cn('text-[10px] px-2 py-1 rounded-full uppercase tracking-wide', statusBadgeClass(selectedSubagent.status))}>
                {selectedSubagent.status}
              </span>
              {selectedSubagentRunningRunId && onCancelRun && (
                <button
                  onClick={() => onCancelRun(selectedSubagentRunningRunId)}
                  disabled={!!cancellingRunIds?.[selectedSubagentRunningRunId]}
                  className={cn(
                    'px-2 py-1 rounded-lg text-[10px] font-semibold border transition-colors',
                    cancellingRunIds?.[selectedSubagentRunningRunId]
                      ? 'bg-slate-100 text-slate-400 border-slate-200 cursor-not-allowed'
                      : 'bg-red-50 text-red-600 border-red-200 hover:bg-red-100'
                  )}
                  title={selectedSubagentRunningRunId}
                >
                  {cancellingRunIds?.[selectedSubagentRunningRunId] ? 'Cancelling...' : 'Cancel Run'}
                </button>
              )}
              <button
                onClick={() => setOpenSubagentId(null)}
                className="p-1.5 rounded-lg hover:bg-slate-100 dark:hover:bg-white/10 text-slate-500"
                title="Close"
              >
                <X size={14} />
              </button>
            </div>
          </div>

          <div className="p-4 border-b border-slate-200 dark:border-white/10">
            <div className="text-[10px] uppercase tracking-widest text-slate-500 mb-2">Active Paths</div>
            <div className="space-y-1 max-h-28 overflow-auto custom-scrollbar">
              {selectedSubagent.paths.map((path) => (
                <div key={path} className="text-[11px] font-mono text-slate-600 dark:text-slate-300 truncate">
                  {path}
                </div>
              ))}
            </div>
          </div>

          <div className="flex-1 overflow-auto p-4 space-y-3 custom-scrollbar">
            <div className="text-[10px] uppercase tracking-widest text-slate-500">Messages</div>
            {selectedSubagentRunId && (
              <div className="rounded-lg border border-slate-200 dark:border-white/10 bg-slate-50 dark:bg-white/5 px-2.5 py-2 text-[10px] text-slate-500 dark:text-slate-400">
                <div className="font-mono text-slate-600 dark:text-slate-300 break-all">{selectedSubagentRunId}</div>
                {selectedSubagentRunOptions.length > 1 && (
                  <div className="mt-1">
                    <select
                      value={selectedSubagentRunId}
                      onChange={(e) => {
                        const runId = e.target.value;
                        if (!selectedSubagentKey) return;
                        setSelectedSubagentRunById((prev) => ({ ...prev, [selectedSubagentKey]: runId }));
                        setPinnedSubagentRunById((prev) => ({ ...prev, [selectedSubagentKey]: true }));
                      }}
                      className="w-full text-[10px] bg-white dark:bg-black/30 border border-slate-200 dark:border-white/10 rounded px-2 py-1 outline-none"
                      title="Select subagent run context"
                    >
                      {selectedSubagentRunOptions.map((run) => (
                        <option key={run.run_id} value={run.run_id}>
                          {formatRunLabel(run)}
                        </option>
                      ))}
                    </select>
                  </div>
                )}
                {selectedSubagentKey && selectedSubagentRunId && (
                  <div className="mt-1">
                    <button
                      onClick={() => {
                        if (selectedSubagentPinned) {
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
                        } else {
                          setSelectedSubagentRunById((prev) => ({ ...prev, [selectedSubagentKey]: selectedSubagentRunId }));
                          setPinnedSubagentRunById((prev) => ({ ...prev, [selectedSubagentKey]: true }));
                        }
                      }}
                      className={cn(
                        'px-2 py-1 rounded border text-[10px] font-semibold',
                        selectedSubagentPinned
                          ? 'bg-slate-100 text-slate-600 border-slate-300'
                          : 'bg-blue-50 text-blue-600 border-blue-200'
                      )}
                    >
                      {selectedSubagentPinned ? 'Unpin' : 'Pin'}
                    </button>
                  </div>
                )}
                {selectedSubagentContext?.summary && (
                  <div className="mt-1">
                    messages: {selectedSubagentContext.summary.message_count} • user: {selectedSubagentContext.summary.user_messages} • agent: {selectedSubagentContext.summary.agent_messages} • system: {selectedSubagentContext.summary.system_messages}
                  </div>
                )}
                {selectedSubagentContextLoading && <div className="mt-1 text-blue-500">Loading context...</div>}
                {selectedSubagentContextError && <div className="mt-1 text-red-500">Context error: {selectedSubagentContextError}</div>}
                {selectedSubagentChildrenLoading && <div className="mt-1 text-blue-500">Loading child runs...</div>}
                {selectedSubagentChildrenError && <div className="mt-1 text-red-500">Children error: {selectedSubagentChildrenError}</div>}
              </div>
            )}
            <div className="rounded-lg border border-slate-200 dark:border-white/10 bg-slate-50/70 dark:bg-white/[0.03] px-2.5 py-2 space-y-2">
              <div className="flex items-center justify-between gap-2">
                <div className="text-[10px] uppercase tracking-widest text-slate-500">Context Tools</div>
                <input
                  value={subagentMessageFilter}
                  onChange={(e) => setSubagentMessageFilter(e.target.value)}
                  placeholder="Filter messages"
                  className="w-40 text-[11px] bg-white dark:bg-black/30 border border-slate-200 dark:border-white/10 rounded px-2 py-1 outline-none"
                />
              </div>
              {selectedSubagentTimeline.length > 0 && (
                <details>
                  <summary className="cursor-pointer text-[11px] font-semibold text-slate-600 dark:text-slate-300">
                    Timeline ({selectedSubagentTimeline.length})
                  </summary>
                  <div className="mt-1.5 space-y-1.5 max-h-28 overflow-auto custom-scrollbar">
                    {selectedSubagentTimeline.map((evt, idx) => (
                      <div key={`${evt.ts}-${evt.label}-${idx}`} className="text-[11px] text-slate-600 dark:text-slate-300">
                        <span className="font-mono text-[10px] text-slate-500 mr-2">{formatTs(evt.ts)}</span>
                        <span className="font-semibold">{evt.label}</span>
                        {evt.detail && <span className="text-slate-500"> • {evt.detail}</span>}
                      </div>
                    ))}
                  </div>
                </details>
              )}
            </div>
            {filteredSubagentMessages.length === 0 && (
              <div className="text-xs italic text-slate-500">No context messages captured for this subagent yet.</div>
            )}
            {filteredSubagentMessages.slice(-20).map((msg, idx) => (
              <div key={`${msg.timestamp}-${idx}-${msg.from || msg.role}`} className="rounded-lg border border-slate-200 dark:border-white/10 bg-slate-50 dark:bg-white/5 p-2.5">
                <div className="text-[9px] uppercase tracking-wider text-slate-500 mb-1">
                  {(msg.from || msg.role).toUpperCase()} {msg.to ? `→ ${msg.to.toUpperCase()}` : ''}
                </div>
                <div className="text-[12px] text-slate-700 dark:text-slate-200 break-words">
                  <MarkdownContent text={visibleMessageText(msg)} />
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </section>
  );
};

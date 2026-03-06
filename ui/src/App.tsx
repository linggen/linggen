import React, { useCallback, useMemo, useState, useEffect, useRef } from 'react';
import { Bot, FilePenLine, Plus, RefreshCw, Settings, Sparkles, Target, Zap } from 'lucide-react';
import { AgentsCard } from './components/AgentsCard';
import { SessionNav } from './components/SessionNav';
import { ModelsCard } from './components/ModelsCard';
import { CollapsibleCard } from './components/CollapsibleCard';
import { SkillsCard } from './components/SkillsCard';
import { FilePreview } from './components/FilePreview';
import { ChatPanel } from './components/chat';
import { HeaderBar } from './components/HeaderBar';
import { SettingsPage } from './components/SettingsPage';
import { MissionSidebarCard } from './components/MissionSidebarCard';
import { AgentSpecEditorModal } from './components/AgentSpecEditorModal';
import type {
  AgentInfo,
  AgentTreeItem,
  AgentRunInfo,
  AgentRunSummary,
  ChatMessage,
  WorkspaceState,
  ModelInfo,
  OllamaPsResponse,
  ProjectInfo,
  QueuedChatItem,
  SessionInfo,
  SkillInfo,
} from './types';
import {
  stripEmbeddedStructuredJson,
  buildAgentWorkInfo,
  buildSubagentInfos,
  shouldHideInternalChatMessage,
  isPersistedToolOnlyMessage,
  reconstructContentFromText,
} from './lib/messageUtils';
import { useChatMessages } from './hooks/useChatMessages';
import { useAgentActivity } from './hooks/useAgentActivity';
import { useSseConnection } from './hooks/useSseConnection';
import { useSseDispatch } from './hooks/useSseDispatch';

const SELECTED_AGENT_STORAGE_KEY = 'linggen:selected-agent';
const VERBOSE_MODE_STORAGE_KEY = 'linggen:verbose-mode';
const SELECTED_PROJECT_STORAGE_KEY = 'linggen:selected-project';
const ACTIVE_SESSION_STORAGE_KEY = 'linggen:active-session';

type Page = 'main' | 'settings';

const compactParams = new URLSearchParams(window.location.search);
const isCompact = compactParams.get('mode') === 'compact';
const compactProject = compactParams.get('project') || '';

const App: React.FC = () => {
  const [currentPage, setCurrentPage] = useState<Page>('main');
  const [initialSettingsTab, setInitialSettingsTab] = useState<import('./types').ManagementTab | undefined>(undefined);
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [selectedProjectRoot, setSelectedProjectRoot] = useState<string>(() => {
    if (typeof window === 'undefined') return '';
    return window.localStorage.getItem(SELECTED_PROJECT_STORAGE_KEY) || '';
  });
  const [agentTree, setAgentTree] = useState<Record<string, AgentTreeItem>>({});
  const [agentTreesByProject, setAgentTreesByProject] = useState<Record<string, Record<string, AgentTreeItem>>>({});
  const [newProjectPath, setNewProjectPath] = useState('');
  const [showAddProject, setShowAddProject] = useState(false);

  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [ollamaStatus, setOllamaStatus] = useState<OllamaPsResponse | null>(null);
  const [defaultModels, setDefaultModels] = useState<string[]>([]);

  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(() => {
    if (typeof window === 'undefined') return null;
    return window.localStorage.getItem(ACTIVE_SESSION_STORAGE_KEY) || null;
  });
  const [sessionCountsByProject, setSessionCountsByProject] = useState<Record<string, number>>({});

  const [, setLogs] = useState<string[]>([]);
  const { chatMessages, displayMessages, chatDispatch, chatEndRef, clearChat: clearChatMessages, isInClearCooldown, scrollToBottom } = useChatMessages();
  const [selectedAgent, setSelectedAgent] = useState<string>(() => {
    if (typeof window === 'undefined') return '';
    const stored = window.localStorage.getItem(SELECTED_AGENT_STORAGE_KEY);
    return stored || '';
  });
  const {
    agentStatus, setAgentStatus, agentStatusText, setAgentStatusText,
    agentContext, setAgentContext, tokensPerSec, isRunning,
    runStartTsRef, latestContextTokensRef, subagentParentMapRef, subagentStatsRef,
    resetStatus,
  } = useAgentActivity(chatMessages);
  const [queuedMessages, setQueuedMessages] = useState<QueuedChatItem[]>([]);
  const [overlay, setOverlay] = useState<string | null>(null);
  const [modelPickerOpen, setModelPickerOpen] = useState(false);
  const [agentRuns, setAgentRuns] = useState<AgentRunInfo[]>([]);
  const [cancellingRunIds, setCancellingRunIds] = useState<Record<string, boolean>>({});
  // Refresh icon should only refresh UI state, not run an audit skill.
  
  const [currentPath, setCurrentPath] = useState('');
  const [selectedFileContent, setSelectedFileContent] = useState<string | null>(null);
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  const [showAgentSpecEditor, setShowAgentSpecEditor] = useState(false);
  
  const [workspaceState, setWorkspaceState] = useState<WorkspaceState | null>(null);
  const [pendingPlan, setPendingPlan] = useState<import('./types').Plan | null>(null);
  const [pendingPlanAgentId, setPendingPlanAgentId] = useState<string | null>(null);
  const [pendingAskUser, setPendingAskUser] = useState<import('./types').PendingAskUser | null>(null);
  const [activePlan, setActivePlan] = useState<import('./types').Plan | null>(null);
  const [verboseMode, setVerboseMode] = useState<boolean>(() => {
    if (typeof window === 'undefined') return false;
    return window.localStorage.getItem(VERBOSE_MODE_STORAGE_KEY) === 'true';
  });

  const prevSessionIdRef = useRef<string | null>(null);
  const mainAgents = agents;
  // Determine which model is currently generating (for token rate display).
  // Extract from status text like "Thinking (gemini-2.5-flash)" since agent.model
  // may be "inherit" or null when using the default routing chain.
  const activeModelId = useMemo(() => {
    for (const name of Object.keys(agentStatusText)) {
      const status = agentStatus[name];
      if (status && status !== 'idle') {
        const text = agentStatusText[name] || '';
        const match = text.match(/\(([^)]+)\)/);
        if (match) return match[1];
      }
    }
    return undefined;
  }, [agentStatus, agentStatusText]);
  const mainAgentIds = useMemo(() => {
    return agents.map((agent) => agent.name.toLowerCase());
  }, [agents]);
  const agentWork = useMemo(() => buildAgentWorkInfo(agentTree), [agentTree]);
  const subagents = useMemo(
    () => buildSubagentInfos(agentTree, mainAgentIds, agentStatus),
    [agentTree, mainAgentIds, agentStatus]
  );
  const sortedAgentRuns = useMemo(() => {
    const statusScore = (status: string) => (status === 'running' ? 1 : 0);
    return [...agentRuns].sort(
      (a, b) => statusScore(b.status) - statusScore(a.status) || Number(b.started_at || 0) - Number(a.started_at || 0)
    );
  }, [agentRuns]);
  const mainRunIds = useMemo(() => {
    const out: Record<string, string> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (run.parent_run_id) continue;
      if (!out[agentId]) out[agentId] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);
  const subagentRunIds = useMemo(() => {
    const out: Record<string, string> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (!run.parent_run_id) continue;
      if (!out[agentId]) out[agentId] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);
  const runningMainRunIds = useMemo(() => {
    const out: Record<string, string> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (run.parent_run_id || run.status !== 'running') continue;
      if (!out[agentId]) out[agentId] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);
  const runningSubagentRunIds = useMemo(() => {
    const out: Record<string, string> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (!run.parent_run_id || run.status !== 'running') continue;
      if (!out[agentId]) out[agentId] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);
  const mainRunHistory = useMemo(() => {
    const out: Record<string, AgentRunInfo[]> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (run.parent_run_id) continue;
      if (!out[agentId]) out[agentId] = [];
      out[agentId].push(run);
    }
    return out;
  }, [sortedAgentRuns]);
  const subagentRunHistory = useMemo(() => {
    const out: Record<string, AgentRunInfo[]> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (!run.parent_run_id) continue;
      if (!out[agentId]) out[agentId] = [];
      out[agentId].push(run);
    }
    return out;
  }, [sortedAgentRuns]);
  const agentRunSummary = useMemo(() => {
    const out: Record<string, AgentRunSummary> = {};
    for (const agent of agents) {
      const agentId = agent.name.toLowerCase();
      const latest = mainRunHistory[agentId]?.[0];
      if (!latest) continue;
      const children = sortedAgentRuns.filter((run) => run.parent_run_id === latest.run_id);
      const timelineEvents =
        1 +
        (latest.ended_at ? 1 : 0) +
        children.reduce((count, child) => count + 1 + (child.ended_at ? 1 : 0), 0);
      const lastEventAt = Math.max(
        Number(latest.ended_at || 0),
        Number(latest.started_at || 0),
        ...children.flatMap((child) => [Number(child.started_at || 0), Number(child.ended_at || 0)])
      );
      out[agentId] = {
        run_id: latest.run_id,
        status: latest.status,
        started_at: latest.started_at,
        ended_at: latest.ended_at,
        child_count: children.length,
        timeline_events: timelineEvents,
        last_event_at: lastEventAt,
      };
    }
    return out;
  }, [agents, mainRunHistory, sortedAgentRuns]);

  const addLog = useCallback((msg: string) => {
    setLogs(prev => [...prev, `[${new Date().toLocaleTimeString()}] ${msg}`]);
  }, []);


  const fetchProjects = useCallback(async () => {
    try {
      const resp = await fetch('/api/projects');
      const data = await resp.json();
      setProjects(data);
      setAgentTreesByProject((prev) => {
        const valid = new Set<string>(data.map((p: ProjectInfo) => p.path));
        const next: Record<string, Record<string, AgentTreeItem>> = {};
        Object.entries(prev).forEach(([path, tree]) => {
          if (valid.has(path)) next[path] = tree;
        });
        return next;
      });
      setSelectedProjectRoot((prev) => {
        if (data.length === 0) return '';
        if (prev && data.some((p: ProjectInfo) => p.path === prev)) return prev;
        return data[0].path;
      });
    } catch (e) {
      addLog(`Error fetching projects: ${e}`);
    }
  }, [addLog]);

  const addProject = async () => {
    if (!newProjectPath.trim()) return;
    try {
      await fetch('/api/projects', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path: newProjectPath }),
      });
      setNewProjectPath('');
      setShowAddProject(false);
      fetchProjects();
    } catch (e) {
      addLog(`Error adding project: ${e}`);
    }
  };

  const removeProject = async (path: string) => {
    if (!confirm(`Are you sure you want to remove project: ${path}?`)) return;
    try {
      await fetch('/api/projects', {
        method: 'DELETE',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path }),
      });
      fetchProjects();
    } catch (e) {
      addLog(`Error removing project: ${e}`);
    }
  };

  const pickFolder = async () => {
    try {
      const resp = await fetch('/api/utils/pick-folder');
      if (resp.ok) {
        const data = await resp.json();
        if (data.path) {
          // Directly add the project after folder selection
          try {
            await fetch('/api/projects', {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({ path: data.path }),
            });
            setNewProjectPath('');
            setShowAddProject(false);
            fetchProjects();
          } catch (e) {
            addLog(`Error adding project: ${e}`);
          }
        }
      } else if (resp.status === 204) {
        // User cancelled the folder picker — do nothing
      } else if (resp.status === 501) {
        addLog("Folder picker not supported on this platform.");
      }
    } catch (e) {
      addLog(`Error picking folder: ${e}`);
    }
  };

  const fetchAgentTree = useCallback(async (projectRoot?: string) => {
    const root = projectRoot || selectedProjectRoot;
    if (!root) return;
    try {
      const resp = await fetch(`/api/workspace/tree?project_root=${encodeURIComponent(root)}`);
      const data = await resp.json();
      setAgentTreesByProject((prev) => ({ ...prev, [root]: data }));
      if (root === selectedProjectRoot || !projectRoot) {
        setAgentTree(data);
      }
    } catch (e) {
      addLog(`Error fetching agent tree (${root}): ${e}`);
    }
  }, [selectedProjectRoot, addLog]);

  const fetchAllAgentTrees = useCallback(async () => {
    if (projects.length === 0) return;
    await Promise.all(projects.map((project) => fetchAgentTree(project.path)));
  }, [projects, fetchAgentTree]);

  const fetchFiles = useCallback(async (path = '') => {
    if (!selectedProjectRoot) return;
    try {
      const resp = await fetch(`/api/files?project_root=${encodeURIComponent(selectedProjectRoot)}&path=${encodeURIComponent(path)}`);
      await resp.json();
      setCurrentPath(path);
    } catch (e) {
      addLog(`Error fetching files: ${e}`);
    }
  }, [selectedProjectRoot, addLog]);

  const readFile = async (path: string, projectRootOverride?: string) => {
    const root = projectRootOverride || selectedProjectRoot;
    if (!root) return;
    try {
      const resp = await fetch(`/api/file?project_root=${encodeURIComponent(root)}&path=${encodeURIComponent(path)}`);
      const data = await resp.json();
      setSelectedFileContent(data.content);
      setSelectedFilePath(path);
    } catch (e) {
      addLog(`Error reading file: ${e}`);
    }
  };

  const selectAgentPathFromTree = (projectRoot: string, path: string) => {
    if (projectRoot !== selectedProjectRoot) {
      setSelectedProjectRoot(projectRoot);
    }
    readFile(path, projectRoot);
  };

  const closeFilePreview = () => {
    setSelectedFilePath(null);
    setSelectedFileContent(null);
  };

  const fetchWorkspaceState = useCallback(async () => {
    if (!selectedProjectRoot || !activeSessionId) return;
    try {
      const url = new URL('/api/workspace/state', window.location.origin);
      url.searchParams.append('project_root', selectedProjectRoot);
      if (activeSessionId) url.searchParams.append('session_id', activeSessionId);
      
      const resp = await fetch(url.toString());
      const data = await resp.json();
      setWorkspaceState(data);
      
      // Update chat messages from state if needed
      // Skip repopulation during clear-chat cooldown (5s) to let server process the clear
      if (data.messages && !isInClearCooldown()) {
        const msgs: ChatMessage[] = data.messages
          .filter(([meta, body]: any) => !shouldHideInternalChatMessage(meta.from, body))
          .filter(([_meta, body]: any) => !isPersistedToolOnlyMessage(String(body || '')))
          .flatMap(([meta, body]: any) => {
            const isUser = meta.from === 'user';
            let bodyStr = String(body || '');

            try {
              const parsed = JSON.parse(bodyStr);
              if (parsed?.type === 'plan' && parsed?.plan) {
                return [{
                  role: 'agent' as const,
                  from: meta.from,
                  to: meta.to,
                  text: bodyStr,
                  timestamp: new Date(meta.ts * 1000).toLocaleTimeString(),
                  timestampMs: Number(meta.ts || 0) * 1000,
                }];
              }
            } catch { /* not pure JSON */ }

            if (!isUser) {
              bodyStr = stripEmbeddedStructuredJson(bodyStr);
            }
            if (!isUser && !bodyStr) return [];
            // Reconstruct content blocks from tool status lines in persisted text
            const restored = !isUser ? reconstructContentFromText(bodyStr) : null;
            const isError = !isUser && bodyStr.startsWith('Error:');
            return [{
              role:
                meta.from === 'user'
                  ? 'user'
                  : 'agent',
              from: meta.from,
              to: meta.to,
              text: bodyStr,
              timestamp: new Date(meta.ts * 1000).toLocaleTimeString(),
              timestampMs: Number(meta.ts || 0) * 1000,
              ...(restored ? { content: restored.content, toolCount: restored.toolCount } : {}),
              ...(isError ? { isError: true } : {}),
            }];
          });
        chatDispatch({ type: 'SYNC_PERSISTED', persisted: msgs });
      }
    } catch (e) {
      addLog(`Error fetching workspace state: ${e}`);
    }
  }, [selectedProjectRoot, activeSessionId, addLog]);

  const fetchSessions = useCallback(async () => {
    if (!selectedProjectRoot) return;
    try {
      const resp = await fetch(`/api/sessions?project_root=${encodeURIComponent(selectedProjectRoot)}`);
      const data: SessionInfo[] = await resp.json();
      setSessions(data);
      // Auto-select the most recent session when none is active
      setActiveSessionId((prev) => {
        if (prev && data.some((s: SessionInfo) => s.id === prev)) return prev;
        if (data.length > 0) return data[0].id;
        return null;
      });
    } catch (e) {
      console.error('Failed to fetch sessions:', e);
    }
  }, [selectedProjectRoot]);

  const createSession = async () => {
    if (!selectedProjectRoot) return;
    const now = new Date();
    const title = `Chat ${now.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}, ${now.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' })}`;

    try {
      const resp = await fetch('/api/sessions', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: selectedProjectRoot, title }),
      });
      const data = await resp.json();
      setActiveSessionId(data.id);
      fetchSessions();
      fetchAllSessionCounts();
    } catch (e) {
      addLog(`Error creating session: ${e}`);
    }
  };

  const removeSession = async (id: string) => {
    if (!selectedProjectRoot || !confirm("Remove this session?")) return;
    try {
      await fetch('/api/sessions', {
        method: 'DELETE',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: selectedProjectRoot, session_id: id }),
      });
      if (activeSessionId === id) setActiveSessionId(null);
      fetchSessions();
      fetchAllSessionCounts();
    } catch (e) {
      addLog(`Error removing session: ${e}`);
    }
  };

  const renameSession = async (id: string, title: string) => {
    if (!selectedProjectRoot) return;
    try {
      await fetch('/api/sessions', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: selectedProjectRoot, session_id: id, title }),
      });
      fetchSessions();
    } catch (e) {
      addLog(`Error renaming session: ${e}`);
    }
  };

  const fetchAllSessionCounts = useCallback(async () => {
    if (projects.length === 0) return;
    const counts: Record<string, number> = {};
    await Promise.all(
      projects.map(async (project) => {
        try {
          const resp = await fetch(`/api/sessions?project_root=${encodeURIComponent(project.path)}`);
          const data = await resp.json();
          counts[project.path] = Array.isArray(data) ? data.length : 0;
        } catch {
          counts[project.path] = 0;
        }
      }),
    );
    setSessionCountsByProject(counts);
  }, [projects]);

  const fetchSkills = useCallback(async () => {
    try {
      const resp = await fetch('/api/skills');
      const data = await resp.json();
      setSkills(data);
    } catch (e) {
      console.error('Failed to fetch skills:', e);
    }
  }, []);

  const [reloadingSkills, setReloadingSkills] = useState(false);
  const reloadSkills = useCallback(async () => {
    setReloadingSkills(true);
    try {
      await fetch('/api/skills/reload', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: selectedProjectRoot || undefined }),
      });
      await fetchSkills();
    } catch (e) {
      console.error('Failed to reload skills:', e);
    } finally {
      setReloadingSkills(false);
    }
  }, [selectedProjectRoot, fetchSkills]);

  const fetchAgents = useCallback(async (projectRootOverride?: string) => {
    const root = projectRootOverride || selectedProjectRoot;
    if (!root) {
      setAgents([]);
      return;
    }
    try {
      const resp = await fetch(`/api/agents?project_root=${encodeURIComponent(root)}`);
      const data = await resp.json();
      setAgents(data);
    } catch (e) {
      console.error('Failed to fetch agents:', e);
    }
  }, [selectedProjectRoot]);

  const fetchModels = useCallback(async () => {
    try {
      const resp = await fetch('/api/models');
      const data = await resp.json();
      setModels(data);
    } catch (e) {
      console.error('Failed to fetch models:', e);
    }
  }, []);

  const fetchDefaultModels = useCallback(async () => {
    try {
      const resp = await fetch('/api/config');
      if (resp.ok) {
        const data = await resp.json();
        setDefaultModels(data.routing?.default_models ?? []);
      }
    } catch { /* ignore */ }
  }, []);

  const toggleDefaultModel = useCallback(async (modelId: string) => {
    try {
      const resp = await fetch('/api/config');
      if (!resp.ok) return;
      const config = await resp.json();
      const current: string[] = config.routing?.default_models ?? [];
      // Single-select: set as default, or deselect if already the default
      const newDefaults = current.length === 1 && current[0] === modelId ? [] : [modelId];
      const updated = { ...config, routing: { ...config.routing, default_models: newDefaults } };
      const saveResp = await fetch('/api/config', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(updated),
      });
      if (saveResp.ok) {
        setDefaultModels(newDefaults);
      }
    } catch { /* ignore */ }
  }, []);

  const fetchOllamaStatus = useCallback(async () => {
    try {
      const resp = await fetch('/api/utils/ollama-status');
      if (resp.ok) {
        const data = await resp.json();
        setOllamaStatus(data);
      }
    } catch (e) {
      console.error('Failed to fetch Ollama status:', e);
    }
  }, []);

  const fetchAgentRuns = useCallback(async () => {
    if (!selectedProjectRoot) return;
    if (!activeSessionId) { setAgentRuns([]); return; }
    try {
      const url = new URL('/api/agent-runs', window.location.origin);
      url.searchParams.append('project_root', selectedProjectRoot);
      url.searchParams.append('session_id', activeSessionId);
      const resp = await fetch(url.toString());
      if (!resp.ok) return;
      const data = await resp.json();
      setAgentRuns(Array.isArray(data) ? data : []);
    } catch (e) {
      addLog(`Error fetching agent runs: ${e}`);
    }
  }, [selectedProjectRoot, activeSessionId, addLog]);

  const cancelAgentRun = async (runId: string) => {
    if (!runId) return;
    setCancellingRunIds((prev) => ({ ...prev, [runId]: true }));
    try {
      await fetch('/api/agent-cancel', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ run_id: runId }),
      });
      await Promise.all([fetchAgentRuns(), fetchWorkspaceState(), fetchAllAgentTrees()]);
    } catch (e) {
      addLog(`Error cancelling run ${runId}: ${e}`);
    } finally {
      setCancellingRunIds((prev) => {
        const next = { ...prev };
        delete next[runId];
        return next;
      });
    }
  };

  useEffect(() => {
    fetchProjects();
    fetchSkills();
    fetchAgents();
    fetchModels();
    fetchDefaultModels();

    const interval = setInterval(fetchOllamaStatus, 5000);
    fetchOllamaStatus();
    return () => clearInterval(interval);
  }, [fetchProjects, fetchSkills, fetchAgents, fetchModels, fetchDefaultModels, fetchOllamaStatus]);

  useEffect(() => {
    if (selectedProjectRoot) {
      fetchFiles();
      fetchWorkspaceState();
      fetchAgentTree(selectedProjectRoot);
      fetchAgentRuns();
      fetchSessions();
      fetchAgents(selectedProjectRoot);
      resetStatus();
      setQueuedMessages([]);
      setActivePlan(null);
    }
  }, [selectedProjectRoot, fetchFiles, fetchWorkspaceState, fetchAgentTree, fetchAgentRuns, fetchSessions, fetchAgents, resetStatus]);

  useEffect(() => {
    if (projects.length === 0) return;
    fetchAllAgentTrees();
    fetchAllSessionCounts();
  }, [projects, fetchAllAgentTrees, fetchAllSessionCounts]);

  useEffect(() => {
    if (selectedProjectRoot) {
      const prev = prevSessionIdRef.current;
      prevSessionIdRef.current = activeSessionId;
      // When adopting an auto-created session (null → id), don't clear messages —
      // they were just added optimistically and the session didn't truly change.
      const isSessionAdoption = prev === null && activeSessionId !== null;
      if (!isSessionAdoption) {
        // Clear stale chat state before fetching the new session's data.
        chatDispatch({ type: 'CLEAR' });
        setQueuedMessages([]);
        setActivePlan(null);
      }
      fetchWorkspaceState();
      fetchAgentRuns();
    }
  }, [activeSessionId, selectedProjectRoot, fetchWorkspaceState, fetchAgentRuns]);


  // In compact mode, force-select the project from the query param
  // and auto-create a dedicated "VS Code" session for it.
  const compactSessionInitRef = useRef(false);
  useEffect(() => {
    if (!isCompact || !compactProject || compactSessionInitRef.current) return;
    compactSessionInitRef.current = true;

    // Force-select the project
    setSelectedProjectRoot(compactProject);

    // Create or reuse a VS Code session for this project
    (async () => {
      try {
        // Fetch existing sessions to see if a "VS Code" session exists
        const resp = await fetch(`/api/sessions?project_root=${encodeURIComponent(compactProject)}`);
        const data = await resp.json();
        const sessionList: SessionInfo[] = data.sessions ?? data ?? [];
        const existing = sessionList.find((s) => s.title?.startsWith('VS Code'));
        if (existing) {
          setActiveSessionId(existing.id);
        } else {
          // Create a new session
          const createResp = await fetch('/api/sessions', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ project_root: compactProject, title: 'VS Code' }),
          });
          const created = await createResp.json();
          setActiveSessionId(created.id);
        }
      } catch (e) {
        addLog(`Compact session init error: ${e}`);
      }
    })();
  }, [compactProject, addLog]);

  useEffect(() => {
    window.localStorage.setItem(SELECTED_AGENT_STORAGE_KEY, selectedAgent);
  }, [selectedAgent]);

  useEffect(() => {
    window.localStorage.setItem(VERBOSE_MODE_STORAGE_KEY, String(verboseMode));
  }, [verboseMode]);

  useEffect(() => {
    if (selectedProjectRoot) {
      window.localStorage.setItem(SELECTED_PROJECT_STORAGE_KEY, selectedProjectRoot);
    } else {
      window.localStorage.removeItem(SELECTED_PROJECT_STORAGE_KEY);
    }
  }, [selectedProjectRoot]);

  useEffect(() => {
    if (activeSessionId) {
      window.localStorage.setItem(ACTIVE_SESSION_STORAGE_KEY, activeSessionId);
    } else {
      window.localStorage.removeItem(ACTIVE_SESSION_STORAGE_KEY);
    }
  }, [activeSessionId]);

  useEffect(() => {
    if (mainAgentIds.length === 0) return;
    if (!mainAgentIds.includes(selectedAgent.toLowerCase())) {
      // Default to 'ling' if available, otherwise first agent
      const preferred = mainAgentIds.includes('ling') ? 'ling' : mainAgentIds[0];
      setSelectedAgent(preferred);
    }
  }, [mainAgentIds, selectedAgent]);

  const handleSseEvent = useSseDispatch({
    chatDispatch,
    setAgentStatus, setAgentStatusText, setAgentContext,
    subagentParentMapRef, subagentStatsRef, runStartTsRef, latestContextTokensRef,
    setQueuedMessages, setPendingAskUser, setActivePlan, setPendingPlan, setPendingPlanAgentId,
    fetchWorkspaceState, fetchFiles, fetchAllAgentTrees, fetchAgentRuns, fetchSessions,
    currentPath, selectedProjectRoot, activeSessionId,
  });

  useSseConnection({
    onEvent: handleSseEvent,
    onParseError: () => { fetchWorkspaceState(); fetchAgentRuns(); },
    sessionId: activeSessionId,
  });

  const sendChatMessage = async (userMessage: string, targetAgent?: string, images?: string[]) => {
    if (!userMessage.trim() && !(images && images.length > 0)) return;
    if (!selectedProjectRoot) return;
    const agentToUse = targetAgent || selectedAgent;
    if (!agentToUse) return;
    const now = new Date();
    const trimmed = userMessage.trim();

    // Commands that show overlays — intercept before adding user message to chat
    if (trimmed === '/help' || trimmed === '/status' || trimmed === '/clear' || trimmed === '/model' || trimmed.startsWith('/model ')) {
      // handled below — don't show as user message
    } else {
      chatDispatch({
        type: 'ADD_MESSAGE',
        message: {
          role: 'user',
          from: 'user',
          to: agentToUse,
          text: userMessage,
          timestamp: now.toLocaleTimeString(),
          timestampMs: now.getTime(),
          isGenerating: false,
          ...(images && images.length > 0 ? { images } : {}),
        },
      });
      // Re-enable auto-scroll and jump to bottom when user sends a message
      scrollToBottom();
    }

    // /model — open model picker overlay; /model <id> — switch model
    if (trimmed === '/model' || trimmed.startsWith('/model ')) {
      const modelArg = trimmed.slice('/model'.length).trim();

      if (!modelArg) {
        // Open interactive model picker
        setModelPickerOpen(true);
        setOverlay(null);
      } else {
        // Switch model directly
        const valid = models.length === 0 || models.some(m => m.id === modelArg);
        if (!valid) {
          setOverlay(`Unknown model: \`${modelArg}\`. Use \`/model\` to see available models.`);
        } else {
          try {
            const resp = await fetch('/api/config');
            if (resp.ok) {
              const config = await resp.json();
              const newDefaults = [modelArg];
              const updated = { ...config, routing: { ...config.routing, default_models: newDefaults } };
              const saveResp = await fetch('/api/config', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(updated),
              });
              if (saveResp.ok) {
                setDefaultModels(newDefaults);
                setOverlay(`Switched default model to: \`${modelArg}\``);
              }
            }
          } catch (e) {
            setOverlay(`Error switching model: ${e}`);
          }
        }
      }
      return;
    }

    // /help — show available commands (overlay, not chat message)
    if (userMessage.trim() === '/help') {
      const helpLines = [
        '**Commands:**',
        '- `/help` — Show available commands',
        '- `/clear` — Clear chat context',
        '- `/status` — Show project status',
        '- `/model` — List models; `/model <id>` — Switch default model',
        '- `/plan <task>` — Ask agent to create a plan (read-only)',
        '- `/image <path>` — Attach an image file',
        '- `@path` — Mention a file',
        '- `@@agent message` — Send to specific agent',
        '',
        '**Skills:** Type `/` to see available skills.',
      ];
      setOverlay(helpLines.join('\n'));
      return;
    }

    // /status — show project status (overlay, not chat message)
    if (userMessage.trim() === '/status') {
      try {
        const resp = await fetch(`/api/status?project_root=${encodeURIComponent(selectedProjectRoot)}`);
        if (resp.ok) {
          const data = await resp.json();
          const defaultModel = data.default_model || '(none)';
          const modelLines = (data.models || []).map((m: { id: string; provider: string; model: string }) => {
            const mark = m.id === data.default_model ? ' ✓' : '';
            return `- \`${m.id}${mark}\`  (${m.provider}: ${m.model})`;
          });
          const usageLines = (data.model_usage || []).map((entry: [string, number]) =>
            `- \`${entry[0]}\` — ${entry[1]} runs`
          );

          const version = data.version || '?';
          const fmt = (n: number) => n >= 1_000_000 ? `${(n / 1_000_000).toFixed(1)}M` : n >= 1_000 ? `${(n / 1_000).toFixed(1)}K` : `${n}`;
          const promptTok = data.session_prompt_tokens || 0;
          const completionTok = data.session_completion_tokens || 0;

          const lines = [
            `**Version:** v${version}`,
            `**Session:** \`${activeSessionId || '(none)'}\``,
            `**Workspace:** \`${selectedProjectRoot}\``,
            `**Agent:** ${selectedAgent}`,
            `**Model:** \`${defaultModel}\``,
          ];

          if (promptTok > 0 || completionTok > 0) {
            lines.push(`**Tokens:** ↑ ${fmt(promptTok)}  ↓ ${fmt(completionTok)}  (total: ${fmt(promptTok + completionTok)})`);
          }

          lines.push(
            '',
            '**Models:**',
            ...modelLines,
            '',
            `| Metric | Value |`,
            `|--------|-------|`,
            `| Sessions | ${data.sessions} |`,
            `| Total runs | ${data.total_runs} |`,
            `| Completed | ${data.completed_runs} |`,
            `| Failed | ${data.failed_runs} |`,
            `| Cancelled | ${data.cancelled_runs} |`,
            `| Active days | ${data.active_days} |`,
          );

          if (usageLines.length > 0) {
            lines.push('', '**Model usage:**', ...usageLines);
          }

          setOverlay(lines.join('\n'));
        } else {
          setOverlay(`Status request failed: ${resp.status} ${resp.statusText}`);
        }
      } catch (e) {
        setOverlay(`Error fetching status: ${e}`);
      }
      return;
    }

    // /clear — clear chat context
    if (userMessage.trim() === '/clear') {
      await clearChat();
      return;
    }

    if (userMessage.startsWith('/user_story ')) {
      const story = userMessage.substring(12).trim();
      addLog(`Setting user story: ${story}`);
      await fetch('/api/task', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ 
          project_root: selectedProjectRoot, 
          agent_id: agentToUse, 
          task: story 
        }),
      });
      return;
    }

    try {
      const resp = await fetch('/api/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          project_root: selectedProjectRoot,
          agent_id: agentToUse,
          message: userMessage,
          session_id: activeSessionId,
          ...(images && images.length > 0 ? { images } : {}),
        }),
      });
      const data = await resp.json();
      // If the server auto-created a session, adopt it
      if (data?.session_id && !activeSessionId) {
        setActiveSessionId(data.session_id);
        fetchSessions();
      }
      if (data?.status === 'queued') {
        // Remove the optimistic user message — queued messages are shown in the
        // queue banner only. They'll appear in chat once the agent processes them.
        chatDispatch({ type: 'REMOVE_LAST_USER_MESSAGE', text: userMessage, agentId: agentToUse });
        return;
      }
      setAgentStatus((prev) => ({ ...prev, [agentToUse]: 'model_loading' }));
      setAgentStatusText((prev) => ({ ...prev, [agentToUse]: 'Model Loading' }));
      chatDispatch({ type: 'UPSERT_GENERATING', agentId: agentToUse, text: 'Model loading...', activityLine: 'Model loading...' });
    } catch (e) {
      addLog(`Error in chat: ${e}`);
    }
  };

  const clearChat = async () => {
    if (!selectedProjectRoot) return;
    // Clear local state immediately — don't wait for API
    clearChatMessages();
    setQueuedMessages([]);
    setActivePlan(null);
    setPendingPlan(null);
    setPendingPlanAgentId(null);
    try {
      const resp = await fetch('/api/chat/clear', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          project_root: selectedProjectRoot,
          session_id: activeSessionId,
        }),
      });
      if (!resp.ok) {
        addLog(`Clear chat API error: ${resp.status}`);
      }
      // Clear again after server confirms — SSE events may have re-added
      // messages while the API call was in flight.
      clearChatMessages();
    } catch (e) {
      addLog(`Error clearing chat: ${e}`);
    }
  };

  const respondToAskUser = async (questionId: string, answers: import('./types').AskUserAnswer[]) => {
    try {
      await fetch('/api/ask-user-response', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ question_id: questionId, answers }),
      });
      setPendingAskUser(null);
    } catch (e) {
      addLog(`Error responding to AskUser: ${e}`);
    }
  };

  const approvePlan = async (clearContext: boolean = false) => {
    if (!pendingPlanAgentId || !selectedProjectRoot) return;
    try {
      await fetch('/api/plan/approve', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          project_root: selectedProjectRoot,
          agent_id: pendingPlanAgentId,
          session_id: activeSessionId,
          clear_context: clearContext,
        }),
      });
      setPendingPlan(null);
      setPendingPlanAgentId(null);
    } catch (e) {
      addLog(`Error approving plan: ${e}`);
    }
  };

  const rejectPlan = async () => {
    if (!pendingPlanAgentId || !selectedProjectRoot) return;
    try {
      await fetch('/api/plan/reject', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          project_root: selectedProjectRoot,
          agent_id: pendingPlanAgentId,
          session_id: activeSessionId,
        }),
      });
      setPendingPlan(null);
      setPendingPlanAgentId(null);
    } catch (e) {
      addLog(`Error rejecting plan: ${e}`);
    }
  };

  const editPlan = async (text: string) => {
    if (!pendingPlanAgentId || !selectedProjectRoot) return;
    try {
      const res = await fetch('/api/plan/edit', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          project_root: selectedProjectRoot,
          agent_id: pendingPlanAgentId,
          text,
        }),
      });
      if (res.ok) {
        setPendingPlan((prev) => prev ? { ...prev, plan_text: text } : prev);
      }
    } catch (e) {
      addLog(`Error editing plan: ${e}`);
    }
  };

  const [copyChatStatus, setCopyChatStatus] = useState<'idle' | 'copied' | 'error'>('idle');

  const copyChat = async () => {
    try {
      const headerLines = [
        `Linggen Agent Chat Export`,
        `Project: ${selectedProjectRoot || '(none)'}`,
        `Session: ${activeSessionId || 'default'}`,
        `Agent: ${selectedAgent}`,
        `ExportedAt: ${new Date().toISOString()}`,
        ``,
      ];

      const body = displayMessages
        .map((m) => {
          const from = m.from || m.role;
          const to = m.to ? ` → ${m.to}` : '';
          const lines: string[] = [`[${m.timestamp}] ${from}${to}`];
          // Include subagent tree entries
          if (m.subagentTree && m.subagentTree.length > 0) {
            for (const sa of m.subagentTree) {
              const stats = [];
              if (sa.toolCount > 0) stats.push(`${sa.toolCount} tool uses`);
              if (sa.contextTokens > 0) stats.push(`${(sa.contextTokens / 1000).toFixed(1)}k tokens`);
              lines.push(`  [subagent:${sa.subagentId}] ${sa.task}${stats.length ? ` (${stats.join(', ')})` : ''} — ${sa.status}`);
            }
          }
          // Include tool activity entries
          const entries = Array.isArray(m.activityEntries) ? m.activityEntries : [];
          if (entries.length > 0) {
            for (const entry of entries) {
              lines.push(`  > ${entry}`);
            }
          } else if (m.activitySummary) {
            lines.push(`  > ${m.activitySummary}`);
          }
          if (m.text) lines.push(m.text);
          return lines.join('\n') + '\n';
        })
        .join('\n');

      const text = headerLines.join('\n') + body;
      await navigator.clipboard.writeText(text);
      setCopyChatStatus('copied');
      window.setTimeout(() => setCopyChatStatus('idle'), 1200);
    } catch (e) {
      console.error('Failed to copy chat', e);
      setCopyChatStatus('error');
      window.setTimeout(() => setCopyChatStatus('idle'), 1600);
    }
  };

  // In compact mode (VS Code sidebar), set a data attribute on <html>
  // so CSS can remap colors to match VS Code's theme.
  // Also bridge clipboard: VS Code intercepts Cmd/Ctrl+C before it reaches
  // the iframe. The wrapper sends postMessage; we listen and use Clipboard API.
  useEffect(() => {
    if (isCompact) {
      document.documentElement.setAttribute('data-compact', '');

      // Handle postMessage from VS Code wrapper for clipboard operations
      const handleMessage = (e: MessageEvent) => {
        if (e.data?.type !== 'linggen-clipboard') return;
        const sel = window.getSelection();
        const text = sel?.toString();
        switch (e.data.action) {
          case 'copy':
            if (text) navigator.clipboard.writeText(text).catch(() => {});
            break;
          case 'cut':
            if (text) {
              navigator.clipboard.writeText(text).catch(() => {});
              document.execCommand('delete');
            }
            break;
          case 'selectAll':
            document.execCommand('selectAll');
            break;
        }
      };
      window.addEventListener('message', handleMessage);

      // Also handle direct keydown in case the event does reach the iframe
      const handleCopy = (e: KeyboardEvent) => {
        if ((e.ctrlKey || e.metaKey) && e.key === 'c' && !e.shiftKey && !e.altKey) {
          const sel = window.getSelection();
          const text = sel?.toString();
          if (text) {
            e.preventDefault();
            navigator.clipboard.writeText(text).catch(() => {});
          }
        }
      };
      document.addEventListener('keydown', handleCopy);
      return () => {
        window.removeEventListener('message', handleMessage);
        document.removeEventListener('keydown', handleCopy);
      };
    }
  }, []);

  return (
    <>
    {!isCompact && currentPage === 'settings' && (
      <SettingsPage
        onBack={() => {
          setCurrentPage('main');
          setInitialSettingsTab(undefined);
          fetchModels();
          fetchDefaultModels();
          fetchOllamaStatus();
        }}
        projectRoot={selectedProjectRoot}
        initialTab={initialSettingsTab}
        missionAgents={agents}
      />
    )}
    <div className={`flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200 font-sans overflow-hidden${currentPage !== 'main' ? ' hidden' : ''}`}>
      {/* Header — hidden in compact mode */}
      {!isCompact && (
        <HeaderBar
          copyChat={copyChat}
          copyChatStatus={copyChatStatus}
          clearChat={clearChat}
          isRunning={isRunning}
          onOpenSettings={() => setCurrentPage('settings')}
        />
      )}

      {/* Compact toolbar — agent selector, session selector, new chat */}
      {isCompact && (
        <div className="flex items-center gap-1.5 px-2 py-1 border-b border-slate-200 dark:border-white/10 bg-white dark:bg-[#0f0f0f] flex-shrink-0">
          <select
            value={selectedAgent}
            onChange={(e) => setSelectedAgent(e.target.value)}
            className="text-xs bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded px-1.5 py-0.5 text-slate-700 dark:text-slate-300 outline-none max-w-[5rem]"
          >
            {agents.map((a) => (
              <option key={a.name} value={a.name}>{a.name}</option>
            ))}
          </select>
          <select
            value={activeSessionId || ''}
            onChange={(e) => setActiveSessionId(e.target.value || null)}
            className="text-xs bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded px-1.5 py-0.5 text-slate-700 dark:text-slate-300 outline-none flex-1 min-w-0 truncate"
          >
            {sessions.length === 0 && <option value="">No sessions</option>}
            {sessions.map((s) => (
              <option key={s.id} value={s.id}>{s.title || s.id.slice(0, 8)}</option>
            ))}
          </select>
          <button
            onClick={createSession}
            title="New chat session"
            className="p-0.5 rounded hover:bg-slate-200 dark:hover:bg-white/10 text-slate-500 hover:text-slate-700 dark:hover:text-slate-300 transition-colors flex-shrink-0"
          >
            <Plus size={14} />
          </button>
          <span className={`text-[10px] flex-shrink-0 ${isRunning ? 'text-green-500' : 'text-slate-400'}`}>
            {isRunning ? 'Running' : 'Idle'}
          </span>
        </div>
      )}

      {/* Main Layout */}
      <div className="flex-1 flex overflow-hidden">

        {/* Left: Sessions + Agents — hidden in compact mode */}
        {!isCompact && (
        <div className="w-72 border-r border-slate-200 dark:border-white/5 flex flex-col bg-white dark:bg-[#0f0f0f] h-full">
          <SessionNav
            projects={projects}
            selectedProjectRoot={selectedProjectRoot}
            setSelectedProjectRoot={setSelectedProjectRoot}
            sessions={sessions}
            activeSessionId={activeSessionId}
            setActiveSessionId={setActiveSessionId}
            createSession={createSession}
            removeSession={removeSession}
            renameSession={renameSession}
            sessionCountsByProject={sessionCountsByProject}
            treesByProject={agentTreesByProject}
            onSelectPath={selectAgentPathFromTree}
            showAddProject={showAddProject}
            setShowAddProject={setShowAddProject}
            newProjectPath={newProjectPath}
            setNewProjectPath={setNewProjectPath}
            addProject={addProject}
            pickFolder={pickFolder}
            removeProject={removeProject}
          />
          <div className="border-t border-slate-200 dark:border-white/5">
            <CollapsibleCard
              title="AGENTS"
              icon={<Bot size={12} />}
              iconColor="text-blue-500"
              badge={`${mainAgents.length}`}
              defaultOpen
              headerAction={
                <button
                  onClick={() => setShowAgentSpecEditor(true)}
                  disabled={!selectedProjectRoot}
                  title="Edit agent markdown specs"
                  className="p-1 rounded hover:bg-slate-200 dark:hover:bg-white/10 text-slate-400 hover:text-slate-600 dark:hover:text-slate-300 disabled:opacity-30 transition-colors"
                >
                  <FilePenLine size={12} />
                </button>
              }
            >
              <AgentsCard
                agents={mainAgents}
                workspaceState={workspaceState}
                isRunning={isRunning}
                selectedAgent={selectedAgent}
                setSelectedAgent={setSelectedAgent}
                agentStatus={agentStatus}
                agentStatusText={agentStatusText}
                agentWork={agentWork}
                agentRunSummary={agentRunSummary}
                agentContext={agentContext}
                projectRoot={selectedProjectRoot}
              />
            </CollapsibleCard>
          </div>
        </div>
        )}

        {/* Center: Chat */}
        <main className={`flex-1 flex flex-col overflow-hidden bg-slate-100/40 dark:bg-[#0a0a0a] min-h-0${isCompact ? ' p-0' : ''}`}>
          <div className={`flex-1 min-h-0${isCompact ? '' : ' p-2'}`}>
            <ChatPanel
              chatMessages={displayMessages}
              queuedMessages={queuedMessages}
              chatEndRef={chatEndRef}
              projectRoot={selectedProjectRoot}
              selectedAgent={selectedAgent}
              setSelectedAgent={setSelectedAgent}
              skills={skills}
              agents={agents}
              mainAgents={mainAgents}
              subagents={subagents}
              mainRunIds={mainRunIds}
              subagentRunIds={subagentRunIds}
              runningMainRunIds={runningMainRunIds}
              runningSubagentRunIds={runningSubagentRunIds}
              mainRunHistory={mainRunHistory}
              subagentRunHistory={subagentRunHistory}
              cancellingRunIds={cancellingRunIds}
              onCancelRun={cancelAgentRun}
              onCancelAgentRun={cancelAgentRun}
              isRunning={isRunning}
              onSendMessage={sendChatMessage}
              pendingPlan={pendingPlan}
              pendingPlanAgentId={pendingPlanAgentId}
              agentContext={agentContext}
              onApprovePlan={approvePlan}
              onRejectPlan={rejectPlan}
              onEditPlan={editPlan}
              pendingAskUser={pendingAskUser}
              onRespondToAskUser={respondToAskUser}
              verboseMode={verboseMode}
              agentStatus={agentStatus}
              overlay={overlay}
              onDismissOverlay={() => { setOverlay(null); setModelPickerOpen(false); }}
              modelPickerOpen={modelPickerOpen}
              models={models}
              defaultModels={defaultModels}
              onSwitchModel={async (modelId: string) => {
                try {
                  const resp = await fetch('/api/config');
                  if (resp.ok) {
                    const config = await resp.json();
                    const newDefaults = [modelId];
                    const updated = { ...config, routing: { ...config.routing, default_models: newDefaults } };
                    const saveResp = await fetch('/api/config', {
                      method: 'POST',
                      headers: { 'Content-Type': 'application/json' },
                      body: JSON.stringify(updated),
                    });
                    if (saveResp.ok) {
                      setDefaultModels(newDefaults);
                      setModelPickerOpen(false);
                      setOverlay(`Switched to: \`${modelId}\``);
                    }
                  }
                } catch (e) {
                  setOverlay(`Error switching model: ${e}`);
                  setModelPickerOpen(false);
                }
              }}
            />
          </div>
        </main>

        {/* Right: Mission, Models, Skills — hidden in compact mode */}
        {!isCompact && (
        <aside className="w-72 border-l border-slate-200 dark:border-white/5 flex flex-col bg-slate-100/40 dark:bg-[#0a0a0a] p-3 gap-3 overflow-y-auto">
          <CollapsibleCard
            title="MISSIONS"
            icon={<Target size={12} />}
            iconColor={'text-slate-400'}
            defaultOpen
          >
            <MissionSidebarCard
              projectRoot={selectedProjectRoot}
              onOpenMission={() => { setInitialSettingsTab('mission'); setCurrentPage('settings'); }}
            />
          </CollapsibleCard>
          <CollapsibleCard
            title="MODELS"
            icon={<Sparkles size={12} />}
            iconColor="text-purple-500"
            badge={`${models.length}`}
            defaultOpen
            headerAction={
              <button
                onClick={() => { setInitialSettingsTab('models'); setCurrentPage('settings'); }}
                className="p-1 hover:bg-slate-100 dark:hover:bg-white/5 rounded transition-colors text-slate-400 hover:text-blue-500"
                title="Manage Models"
              >
                <Settings size={12} />
              </button>
            }
          >
            <ModelsCard
              models={models}
              agents={mainAgents}
              ollamaStatus={ollamaStatus}
              chatMessages={chatMessages}
              tokensPerSec={tokensPerSec}
              activeModelId={activeModelId}
              agentContext={agentContext}
              defaultModels={defaultModels}
              onToggleDefault={toggleDefaultModel}
            />
          </CollapsibleCard>
          <CollapsibleCard
            title="SKILLS"
            icon={<Zap size={12} />}
            iconColor="text-amber-500"
            badge={`${skills.length} loaded`}
            defaultOpen
            headerAction={
              <div className="flex items-center gap-0.5">
                <button
                  onClick={reloadSkills}
                  disabled={reloadingSkills}
                  className="p-1 hover:bg-slate-100 dark:hover:bg-white/5 rounded transition-colors text-slate-400 hover:text-blue-500 disabled:opacity-50"
                  title="Reload skills from disk"
                >
                  <RefreshCw size={12} className={reloadingSkills ? 'animate-spin' : ''} />
                </button>
                <button
                  onClick={() => { setInitialSettingsTab('skills'); setCurrentPage('settings'); }}
                  className="p-1 hover:bg-slate-100 dark:hover:bg-white/5 rounded transition-colors text-slate-400 hover:text-blue-500"
                  title="Manage Skills"
                >
                  <Settings size={12} />
                </button>
              </div>
            }
          >
            <SkillsCard skills={skills} projectRoot={selectedProjectRoot} />
          </CollapsibleCard>
        </aside>
        )}
      </div>

      <FilePreview selectedFilePath={selectedFilePath} selectedFileContent={selectedFileContent} onClose={closeFilePreview} />
      <AgentSpecEditorModal
        open={showAgentSpecEditor}
        projectRoot={selectedProjectRoot}
        onClose={() => setShowAgentSpecEditor(false)}
        onChanged={() => {
          fetchAgents(selectedProjectRoot);
          fetchWorkspaceState();
          fetchAllAgentTrees();
        }}
      />

      <style>{`
        .custom-scrollbar { scrollbar-gutter: stable; }
        .custom-scrollbar::-webkit-scrollbar { width: 8px; }
        .custom-scrollbar::-webkit-scrollbar-track { background: rgba(0, 0, 0, 0.04); }
        .custom-scrollbar::-webkit-scrollbar-thumb { background: rgba(59, 130, 246, 0.45); border-radius: 10px; }
        .custom-scrollbar::-webkit-scrollbar-thumb:hover { background: rgba(59, 130, 246, 0.7); }

        /* VS Code dark theme overrides for compact mode */
        html[data-compact] {
          --vsc-bg: #1e1e1e;
          --vsc-sidebar: #252526;
          --vsc-input: #3c3c3c;
          --vsc-border: #3c3c3c;
          --vsc-fg: #cccccc;
          --vsc-fg-muted: #858585;
          --vsc-accent: #0e639c;
          color-scheme: dark;
        }
        html[data-compact] .dark\\:bg-\\[\\#0a0a0a\\] { background-color: var(--vsc-bg) !important; }
        html[data-compact] .dark\\:bg-\\[\\#0f0f0f\\] { background-color: var(--vsc-sidebar) !important; }
        html[data-compact] .dark\\:bg-white\\/\\[0\\.02\\] { background-color: var(--vsc-sidebar) !important; }
        html[data-compact] .dark\\:bg-white\\/5 { background-color: rgba(255,255,255,0.03) !important; }
        html[data-compact] .dark\\:bg-black\\/20 { background-color: var(--vsc-input) !important; }
        html[data-compact] .dark\\:bg-black\\/30 { background-color: var(--vsc-input) !important; }
        html[data-compact] .dark\\:border-white\\/5,
        html[data-compact] .dark\\:border-white\\/10 { border-color: var(--vsc-border) !important; }
        html[data-compact] section { border-radius: 0 !important; border: none !important; }
        /* Text colors — boost for readability */
        html[data-compact] .dark\\:text-slate-200 { color: var(--vsc-fg) !important; }
        html[data-compact] .dark\\:text-slate-300 { color: #d4d4d4 !important; }
        html[data-compact] .dark\\:text-slate-400 { color: #969696 !important; }
        html[data-compact] .text-slate-500 { color: #969696 !important; }
        html[data-compact] .text-slate-400 { color: #969696 !important; }
        html[data-compact] .text-slate-600 { color: #b0b0b0 !important; }
        html[data-compact] select, html[data-compact] input, html[data-compact] textarea {
          color: var(--vsc-fg) !important;
          background-color: var(--vsc-input) !important;
          border-color: var(--vsc-border) !important;
          font-size: 13px !important;
        }
        html[data-compact] ::placeholder { color: var(--vsc-fg-muted) !important; opacity: 1 !important; }
        /* Font — match VS Code */
        html[data-compact] body {
          font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
          font-size: 13px;
          color: var(--vsc-fg);
        }
        /* Scrollbar — VS Code style */
        html[data-compact] .custom-scrollbar::-webkit-scrollbar-thumb { background: rgba(121,121,121,0.4); border-radius: 0; }
        html[data-compact] .custom-scrollbar::-webkit-scrollbar-thumb:hover { background: rgba(121,121,121,0.7); }
        html[data-compact] .custom-scrollbar::-webkit-scrollbar-track { background: transparent; }

        /* --- Dialog / widget cards (ToolPermission, AskUser, etc.) --- */
        html[data-compact] .dark\\:bg-\\[\\#141414\\] { background-color: var(--vsc-sidebar) !important; }

        /* Amber (permission) card overrides */
        html[data-compact] .dark\\:border-amber-500\\/20,
        html[data-compact] .dark\\:border-amber-500\\/10 { border-color: var(--vsc-border) !important; }
        html[data-compact] .dark\\:bg-amber-500\\/5 { background-color: rgba(255,255,255,0.03) !important; }
        html[data-compact] .dark\\:bg-amber-500\\/10 { background-color: rgba(255,255,255,0.05) !important; }
        html[data-compact] .dark\\:text-amber-400 { color: #cca700 !important; }
        html[data-compact] .dark\\:text-amber-300 { color: #ddb700 !important; }
        html[data-compact] .dark\\:text-amber-300\\/80 { color: rgba(221,183,0,0.8) !important; }
        html[data-compact] .dark\\:hover\\:border-amber-500\\/40:hover { border-color: #cca700 !important; }
        html[data-compact] .dark\\:hover\\:bg-amber-500\\/5:hover { background-color: rgba(255,255,255,0.05) !important; }
        html[data-compact] .dark\\:border-amber-500\\/30 { border-color: rgba(204,167,0,0.4) !important; }

        /* Blue (ask-user) card overrides */
        html[data-compact] .dark\\:border-blue-500\\/20,
        html[data-compact] .dark\\:border-blue-500\\/10 { border-color: var(--vsc-border) !important; }
        html[data-compact] .dark\\:bg-blue-500\\/5 { background-color: rgba(255,255,255,0.03) !important; }
        html[data-compact] .dark\\:text-blue-400 { color: #569cd6 !important; }
        html[data-compact] .dark\\:hover\\:border-blue-500\\/40:hover { border-color: #569cd6 !important; }
        html[data-compact] .dark\\:bg-blue-500 { background-color: var(--vsc-accent) !important; border-color: var(--vsc-accent) !important; }
        html[data-compact] .dark\\:hover\\:bg-blue-600:hover { background-color: #1177bb !important; }

        /* Buttons — match VS Code style */
        html[data-compact] .dark\\:bg-white\\/5 {
          background-color: var(--vsc-input) !important;
          border-color: var(--vsc-border) !important;
        }
        html[data-compact] .dark\\:hover\\:bg-white\\/10:hover { background-color: rgba(255,255,255,0.08) !important; }
        html[data-compact] .dark\\:hover\\:border-red-500\\/40:hover { border-color: #f14c4c !important; }
        html[data-compact] .dark\\:hover\\:text-red-400:hover { color: #f14c4c !important; }
        html[data-compact] .dark\\:hover\\:border-slate-400\\/30:hover { border-color: var(--vsc-fg-muted) !important; }
        html[data-compact] .dark\\:hover\\:text-slate-400:hover { color: #b0b0b0 !important; }
        html[data-compact] .dark\\:hover\\:text-slate-300:hover { color: #d4d4d4 !important; }
        html[data-compact] .dark\\:hover\\:text-amber-400:hover { color: #cca700 !important; }
        html[data-compact] .dark\\:hover\\:border-amber-500\\/30:hover { border-color: rgba(204,167,0,0.4) !important; }

        /* Amber accent buttons (Send) */
        html[data-compact] .dark\\:bg-amber-600 { background-color: #b8860b !important; }
        html[data-compact] .dark\\:hover\\:bg-amber-700:hover { background-color: #996f0a !important; }

        /* Section / card shape — flat VS Code look */
        html[data-compact] section { border-radius: 2px !important; border: 1px solid var(--vsc-border) !important; }
        html[data-compact] .rounded-xl { border-radius: 2px !important; }
        html[data-compact] .rounded-lg { border-radius: 2px !important; }
        html[data-compact] .rounded-md { border-radius: 2px !important; }
        html[data-compact] .shadow-sm { box-shadow: none !important; }
        html[data-compact] .shadow-xl { box-shadow: 0 2px 8px rgba(0,0,0,0.36) !important; }

        /* Status badge colors */
        html[data-compact] .dark\\:text-green-300 { color: #89d185 !important; }
        html[data-compact] .dark\\:text-blue-300 { color: #569cd6 !important; }
        html[data-compact] .dark\\:text-amber-300 { color: #cca700 !important; }
        html[data-compact] .dark\\:text-indigo-300 { color: #b4a0ff !important; }

        /* Disabled state */
        html[data-compact] .dark\\:text-slate-600 { color: #5a5a5a !important; }
        html[data-compact] .dark\\:text-slate-500 { color: #6e6e6e !important; }

        /* Focus rings — VS Code style */
        html[data-compact] .dark\\:focus\\:border-amber-500\\/40:focus { border-color: var(--vsc-accent) !important; }
        html[data-compact] .dark\\:focus\\:border-blue-500\\/40:focus { border-color: var(--vsc-accent) !important; }

        /* Chat input area */
        html[data-compact] .dark\\:bg-white\\/\\[0\\.02\\] { background-color: var(--vsc-sidebar) !important; }

        /* Popup menus (slash commands, mentions, files) */
        html[data-compact] .dark\\:hover\\:bg-white\\/5:hover { background-color: rgba(255,255,255,0.06) !important; }

        /* Markdown body — text and code readability in dark mode */
        html[data-compact] .markdown-body { color: var(--vsc-fg, #ccc) !important; }
        html[data-compact] .markdown-body code { background: rgba(255,255,255,0.1) !important; color: #d4d4d4 !important; }
        html[data-compact] .markdown-body pre { background: rgba(255,255,255,0.06) !important; }
        html[data-compact] .markdown-body pre code { color: #c9d1d9 !important; }
        html[data-compact] .markdown-body th,
        html[data-compact] .markdown-body td { border-color: rgba(255,255,255,0.15) !important; color: #d4d4d4 !important; }
        html[data-compact] .markdown-body th { background: rgba(255,255,255,0.06) !important; }
        html[data-compact] .markdown-body hr { border-top-color: rgba(255,255,255,0.12) !important; }
        html[data-compact] .markdown-body blockquote { border-left-color: rgba(59,130,246,0.5) !important; color: #b4becd !important; }
        html[data-compact] .markdown-body h1,
        html[data-compact] .markdown-body h2,
        html[data-compact] .markdown-body h3,
        html[data-compact] .markdown-body h4 { color: #e0e0e0 !important; }
        /* highlight.js tokens — GitHub Dark in compact mode */
        html[data-compact] .hljs { color: #c9d1d9 !important; background: transparent !important; }
        html[data-compact] .hljs-doctag, html[data-compact] .hljs-keyword,
        html[data-compact] .hljs-template-tag, html[data-compact] .hljs-template-variable,
        html[data-compact] .hljs-type, html[data-compact] .hljs-variable.language_ { color: #ff7b72 !important; }
        html[data-compact] .hljs-title, html[data-compact] .hljs-title.class_,
        html[data-compact] .hljs-title.function_ { color: #d2a8ff !important; }
        html[data-compact] .hljs-attr, html[data-compact] .hljs-attribute,
        html[data-compact] .hljs-literal, html[data-compact] .hljs-meta,
        html[data-compact] .hljs-number, html[data-compact] .hljs-operator,
        html[data-compact] .hljs-variable, html[data-compact] .hljs-selector-attr,
        html[data-compact] .hljs-selector-class, html[data-compact] .hljs-selector-id { color: #79c0ff !important; }
        html[data-compact] .hljs-regexp, html[data-compact] .hljs-string { color: #a5d6ff !important; }
        html[data-compact] .hljs-built_in, html[data-compact] .hljs-symbol { color: #ffa657 !important; }
        html[data-compact] .hljs-comment, html[data-compact] .hljs-code, html[data-compact] .hljs-formula { color: #8b949e !important; }
        html[data-compact] .hljs-name, html[data-compact] .hljs-quote,
        html[data-compact] .hljs-selector-tag, html[data-compact] .hljs-selector-pseudo { color: #7ee787 !important; }
        html[data-compact] .hljs-subst { color: #c9d1d9 !important; }
        html[data-compact] .hljs-addition { color: #aff5b4 !important; background-color: #033a16 !important; }
        html[data-compact] .hljs-deletion { color: #ffdcd7 !important; background-color: #67060c !important; }
      `}</style>
    </div>
    </>
  );
};

export default App;

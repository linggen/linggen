import React, { useCallback, useMemo, useState, useEffect, useRef } from 'react';
import { Bot, FilePenLine, Settings, Sparkles, Target, Zap } from 'lucide-react';
import { AgentsCard } from './components/AgentsCard';
import { SessionNav } from './components/SessionNav';
import { ModelsCard } from './components/ModelsCard';
import { CollapsibleCard } from './components/CollapsibleCard';
import { SkillsCard } from './components/SkillsCard';
import { FilePreview } from './components/FilePreview';
import { ChatPanel } from './components/chat';
import { HeaderBar } from './components/HeaderBar';
import { SettingsPage } from './components/SettingsPage';
import { StoragePage } from './components/StoragePage';
import { MissionPage } from './components/MissionPage';
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
  IdlePromptEvent,
  MissionInfo,
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

const SELECTED_AGENT_STORAGE_KEY = 'linggen-agent:selected-agent';
const VERBOSE_MODE_STORAGE_KEY = 'linggen-agent:verbose-mode';
const SELECTED_PROJECT_STORAGE_KEY = 'linggen-agent:selected-project';
const ACTIVE_SESSION_STORAGE_KEY = 'linggen-agent:active-session';

type Page = 'main' | 'settings' | 'storage' | 'mission';

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
  const { chatMessages, displayMessages, chatDispatch, chatEndRef, clearChat: clearChatMessages, isInClearCooldown } = useChatMessages();
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
  const [agentRuns, setAgentRuns] = useState<AgentRunInfo[]>([]);
  const [cancellingRunIds, setCancellingRunIds] = useState<Record<string, boolean>>({});
  // Refresh icon should only refresh UI state, not run an audit skill.
  
  const [currentPath, setCurrentPath] = useState('');
  const [selectedFileContent, setSelectedFileContent] = useState<string | null>(null);
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  const [showAgentSpecEditor, setShowAgentSpecEditor] = useState(false);
  
  const [workspaceState, setWorkspaceState] = useState<WorkspaceState | null>(null);
  const [mission, setMission] = useState<MissionInfo | null>(null);
  const [missionDraft, setMissionDraft] = useState<string | null>(null);
  const [idlePromptEvents, setIdlePromptEvents] = useState<IdlePromptEvent[]>([]);
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
          setNewProjectPath(data.path);
        }
      } else {
        addLog("Folder picker not supported on this OS yet.");
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
      const newDefaults = current.includes(modelId)
        ? current.filter((id: string) => id !== modelId)
        : [...current, modelId];
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

  const missionEndpointAvailable = useRef<boolean | null>(null);
  const prevMissionProjectRoot = useRef(selectedProjectRoot);

  // Reset endpoint availability when switching projects
  if (selectedProjectRoot !== prevMissionProjectRoot.current) {
    missionEndpointAvailable.current = null;
    prevMissionProjectRoot.current = selectedProjectRoot;
  }

  const fetchMission = useCallback(async () => {
    if (!selectedProjectRoot) return;
    if (missionEndpointAvailable.current === false) return;
    try {
      const url = new URL('/api/mission', window.location.origin);
      url.searchParams.append('project_root', selectedProjectRoot);
      const resp = await fetch(url.toString());
      if (resp.status === 404) {
        missionEndpointAvailable.current = false;
        return;
      }
      missionEndpointAvailable.current = true;
      if (!resp.ok) return;
      const data = await resp.json();
      setMission(data?.text ? data : null);
    } catch {
      // silently ignore — mission endpoint may not be available
    }
  }, [selectedProjectRoot]);

  const saveMission = async (text: string) => {
    if (!selectedProjectRoot || !text.trim()) return;
    try {
      await fetch('/api/mission', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: selectedProjectRoot, text: text.trim(), agents: [] }),
      });
      setMissionDraft(null);
      fetchMission();
    } catch (e) {
      addLog(`Error setting mission: ${e}`);
    }
  };

  const clearMission = async () => {
    if (!selectedProjectRoot) return;
    try {
      await fetch('/api/mission', {
        method: 'DELETE',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: selectedProjectRoot }),
      });
      setMission(null);
    } catch (e) {
      addLog(`Error clearing mission: ${e}`);
    }
  };

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
      fetchMission();
      resetStatus();
      setQueuedMessages([]);
      setActivePlan(null);
    }
  }, [selectedProjectRoot, fetchFiles, fetchWorkspaceState, fetchAgentTree, fetchAgentRuns, fetchSessions, fetchAgents, fetchMission, resetStatus]);

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
    setQueuedMessages, setPendingAskUser, setActivePlan, setPendingPlan, setPendingPlanAgentId, setIdlePromptEvents,
    fetchWorkspaceState, fetchFiles, fetchAllAgentTrees, fetchAgentRuns,
    currentPath, selectedProjectRoot, activeSessionId,
  });

  useSseConnection({
    onEvent: handleSseEvent,
    onParseError: () => { fetchWorkspaceState(); fetchAgentRuns(); },
  });

  const sendChatMessage = async (userMessage: string, targetAgent?: string, images?: string[]) => {
    if (!userMessage.trim() && !(images && images.length > 0)) return;
    if (!selectedProjectRoot) return;
    const agentToUse = targetAgent || selectedAgent;
    if (!agentToUse) return;
    const now = new Date();

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
        // Keep the user message in chat — it has the correct timestamp for
        // time-ordered display. The queue banner still shows it separately.
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

  return (
    <>
    {currentPage === 'settings' && (
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
      />
    )}
    {currentPage === 'storage' && (
      <StoragePage
        onBack={() => setCurrentPage('main')}
      />
    )}
    {currentPage === 'mission' && (
      <MissionPage
        onBack={() => setCurrentPage('main')}
        projectRoot={selectedProjectRoot}
        agents={agents}
        agentStatus={agentStatus}
        agentRunSummary={agentRunSummary}
        mission={mission}
        missionDraft={missionDraft}
        onMissionDraftChange={setMissionDraft}
        onSaveMission={saveMission}
        onClearMission={clearMission}
        idlePromptEvents={idlePromptEvents}
      />
    )}
    <div className={`flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200 font-sans overflow-hidden${currentPage !== 'main' ? ' hidden' : ''}`}>
      {/* Header */}
      <HeaderBar
        copyChat={copyChat}
        copyChatStatus={copyChatStatus}
        clearChat={clearChat}
        isRunning={isRunning}
        onOpenMission={() => setCurrentPage('mission')}
        missionActive={!!mission?.active}
        onOpenStorage={() => setCurrentPage('storage')}
        onOpenSettings={() => setCurrentPage('settings')}
      />

      {/* Main Layout */}
      <div className="flex-1 flex overflow-hidden">

        {/* Left: Sessions + Agents */}
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

        {/* Center: Chat */}
        <main className="flex-1 flex flex-col overflow-hidden bg-slate-100/40 dark:bg-[#0a0a0a] min-h-0">
          <div className="flex-1 p-2 min-h-0">
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
            />
          </div>
        </main>

        {/* Right: Mission, Models, Skills */}
        <aside className="w-72 border-l border-slate-200 dark:border-white/5 flex flex-col bg-slate-100/40 dark:bg-[#0a0a0a] p-3 gap-3 overflow-y-auto">
          <CollapsibleCard
            title="MISSION"
            icon={<Target size={12} />}
            iconColor={mission ? 'text-green-500' : 'text-slate-400'}
            badge={mission ? 'Active' : 'None'}
            defaultOpen
          >
            <MissionSidebarCard
              mission={mission}
              projectRoot={selectedProjectRoot}
              onOpenMission={() => setCurrentPage('mission')}
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
              <button
                onClick={() => { setInitialSettingsTab('skills'); setCurrentPage('settings'); }}
                className="p-1 hover:bg-slate-100 dark:hover:bg-white/5 rounded transition-colors text-slate-400 hover:text-blue-500"
                title="Manage Skills"
              >
                <Settings size={12} />
              </button>
            }
          >
            <SkillsCard skills={skills} projectRoot={selectedProjectRoot} />
          </CollapsibleCard>
        </aside>
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
      `}</style>
    </div>
    </>
  );
};

export default App;

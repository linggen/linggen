/**
 * Agents, models, skills, runs, and live activity state.
 */
import { create } from 'zustand';
import type { AgentInfo, AgentRunInfo, ModelInfo, OllamaPsResponse, SkillInfo } from '../types';
import { useProjectStore } from './projectStore';
import { TOKEN_RATE_WINDOW_MS } from '../lib/messageUtils';

export type AgentStatusValue = 'idle' | 'model_loading' | 'thinking' | 'calling_tool' | 'working';

type StateSetter<T> = T | ((prev: T) => T);

interface AgentState {
  agents: AgentInfo[];
  models: ModelInfo[];
  ollamaStatus: OllamaPsResponse | null;
  defaultModels: string[];
  skills: SkillInfo[];
  agentRuns: AgentRunInfo[];
  selectedAgent: string;
  sessionTokens: { prompt: number; completion: number };
  cancellingRunIds: Record<string, boolean>;
  reloadingSkills: boolean;
  reloadingAgents: boolean;

  // Agent activity (live status)
  agentStatus: Record<string, AgentStatusValue>;
  agentStatusText: Record<string, string>;
  agentContext: Record<string, { tokens: number; messages: number; tokenLimit?: number }>;
  tokensPerSec: number;

  // Non-reactive tracking (not in React state, just plain properties)
  _runStartTs: Record<string, number>;
  _latestContextTokens: Record<string, number>;
  _subagentParentMap: Record<string, string>;
  _subagentStats: Record<string, { toolCount: number; contextTokens: number }>;
  _tokenRateSamples: Array<{ ts: number; tokens: number }>;
  _lastTokenAt: number;

  // Derived
  isRunning: () => boolean;

  // Actions
  setSelectedAgent: (agent: string) => void;
  setAgentStatus: (updater: StateSetter<Record<string, AgentStatusValue>>) => void;
  setAgentStatusText: (updater: StateSetter<Record<string, string>>) => void;
  setAgentContext: (updater: StateSetter<Record<string, { tokens: number; messages: number; tokenLimit?: number }>>) => void;
  resetStatus: () => void;
  recordTokenEvent: () => void;
  recomputeTokenRate: (nowMs?: number) => void;

  fetchAgents: (projectRoot?: string) => Promise<void>;
  fetchModels: () => Promise<void>;
  fetchDefaultModels: () => Promise<void>;
  toggleDefaultModel: (modelId: string) => Promise<void>;
  setReasoningEffort: (modelId: string, effort: string | null) => Promise<void>;
  fetchOllamaStatus: () => Promise<void>;
  fetchSkills: () => Promise<void>;
  reloadSkills: () => Promise<void>;
  reloadAgents: () => Promise<void>;
  fetchAgentRuns: () => Promise<void>;
  cancelAgentRun: (runId: string) => Promise<void>;
  fetchSessionTokens: () => Promise<void>;
}

const SELECTED_AGENT_STORAGE_KEY = 'linggen:selected-agent';

export const useAgentStore = create<AgentState>((set, get) => ({
  agents: [],
  models: [],
  ollamaStatus: null,
  defaultModels: [],
  skills: [],
  agentRuns: [],
  selectedAgent: typeof window !== 'undefined'
    ? window.localStorage.getItem(SELECTED_AGENT_STORAGE_KEY) || ''
    : '',
  sessionTokens: { prompt: 0, completion: 0 },
  cancellingRunIds: {},
  reloadingSkills: false,
  reloadingAgents: false,

  agentStatus: {},
  agentStatusText: {},
  agentContext: {},
  tokensPerSec: 0,

  _runStartTs: {},
  _latestContextTokens: {},
  _subagentParentMap: {},
  _subagentStats: {},
  _tokenRateSamples: [],
  _lastTokenAt: 0,

  isRunning: () => Object.values(get().agentStatus).some((s) => s !== 'idle'),

  setSelectedAgent: (agent) => {
    window.localStorage.setItem(SELECTED_AGENT_STORAGE_KEY, agent);
    set({ selectedAgent: agent });
  },
  setAgentStatus: (updater) => set((s) => ({
    agentStatus: typeof updater === 'function' ? updater(s.agentStatus) : updater,
  })),
  setAgentStatusText: (updater) => set((s) => ({
    agentStatusText: typeof updater === 'function' ? updater(s.agentStatusText) : updater,
  })),
  setAgentContext: (updater) => set((s) => ({
    agentContext: typeof updater === 'function' ? updater(s.agentContext) : updater,
  })),
  resetStatus: () => set({ agentStatus: {}, agentStatusText: {} }),
  recordTokenEvent: () => {
    // Called when a token SSE event arrives — records timestamp for rate calculation
    get()._lastTokenAt = Date.now();
  },
  recomputeTokenRate: (nowMs) => {
    const state = get();
    const now = nowMs ?? Date.now();
    const cutoff = now - TOKEN_RATE_WINDOW_MS;
    state._tokenRateSamples = state._tokenRateSamples.filter((s) => s.ts >= cutoff);
    if (state._tokenRateSamples.length === 0) {
      set({ tokensPerSec: 0 });
      return;
    }
    const totalTokens = state._tokenRateSamples.reduce((sum, s) => sum + s.tokens, 0);
    const oldestTs = state._tokenRateSamples[0]?.ts ?? now;
    const elapsedSec = Math.max((now - oldestTs) / 1000, 0.25);
    const rate = totalTokens / elapsedSec;
    set({ tokensPerSec: Number.isFinite(rate) ? rate : 0 });
  },

  fetchAgents: async (projectRootOverride) => {
    const root = projectRootOverride || useProjectStore.getState().selectedProjectRoot;
    if (!root) { set({ agents: [] }); return; }
    try {
      const resp = await fetch(`/api/agents?project_root=${encodeURIComponent(root)}`);
      const data = await resp.json();
      set({ agents: data });
    } catch (e) {
      console.error('Failed to fetch agents:', e);
    }
  },

  fetchModels: async () => {
    try {
      const resp = await fetch('/api/models');
      set({ models: await resp.json() });
    } catch (e) {
      console.error('Failed to fetch models:', e);
    }
  },

  fetchDefaultModels: async () => {
    try {
      const resp = await fetch('/api/config');
      if (resp.ok) {
        const data = await resp.json();
        set({ defaultModels: data.routing?.default_models ?? [] });
      }
    } catch { /* ignore */ }
  },

  toggleDefaultModel: async (modelId) => {
    try {
      const resp = await fetch('/api/config');
      if (!resp.ok) return;
      const config = await resp.json();
      const current: string[] = config.routing?.default_models ?? [];
      const newDefaults = current.length === 1 && current[0] === modelId ? [] : [modelId];
      const updated = { ...config, routing: { ...config.routing, default_models: newDefaults } };
      const saveResp = await fetch('/api/config', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(updated),
      });
      if (saveResp.ok) set({ defaultModels: newDefaults });
    } catch { /* ignore */ }
  },

  setReasoningEffort: async (modelId, effort) => {
    try {
      const resp = await fetch('/api/config');
      if (!resp.ok) return;
      const config = await resp.json();
      const models = config.models ?? [];
      const idx = models.findIndex((m: { id: string }) => m.id === modelId);
      if (idx === -1) return;
      models[idx] = { ...models[idx], reasoning_effort: effort || null };
      const updated = { ...config, models };
      const saveResp = await fetch('/api/config', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(updated),
      });
      if (saveResp.ok) {
        // Update local models state
        set((state) => ({
          models: state.models.map((m) =>
            m.id === modelId ? { ...m, reasoning_effort: effort || null } : m
          ),
        }));
      }
    } catch { /* ignore */ }
  },

  fetchOllamaStatus: async () => {
    try {
      const resp = await fetch('/api/utils/ollama-status');
      if (resp.ok) set({ ollamaStatus: await resp.json() });
    } catch (e) {
      console.error('Failed to fetch Ollama status:', e);
    }
  },

  fetchSkills: async () => {
    try {
      const resp = await fetch('/api/skills');
      set({ skills: await resp.json() });
    } catch (e) {
      console.error('Failed to fetch skills:', e);
    }
  },

  reloadSkills: async () => {
    set({ reloadingSkills: true });
    try {
      const { selectedProjectRoot } = useProjectStore.getState();
      await fetch('/api/skills/reload', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: selectedProjectRoot || undefined }),
      });
      await get().fetchSkills();
    } catch (e) {
      console.error('Failed to reload skills:', e);
    } finally {
      set({ reloadingSkills: false });
    }
  },

  reloadAgents: async () => {
    set({ reloadingAgents: true });
    try {
      const { selectedProjectRoot } = useProjectStore.getState();
      await fetch('/api/agents/reload', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: selectedProjectRoot || undefined }),
      });
      await get().fetchAgents(selectedProjectRoot || undefined);
    } catch (e) {
      console.error('Failed to reload agents:', e);
    } finally {
      set({ reloadingAgents: false });
    }
  },

  fetchAgentRuns: async () => {
    const { selectedProjectRoot, activeSessionId } = useProjectStore.getState();
    if (!selectedProjectRoot) return;
    if (!activeSessionId) { set({ agentRuns: [] }); return; }
    try {
      const url = new URL('/api/agent-runs', window.location.origin);
      url.searchParams.append('project_root', selectedProjectRoot);
      url.searchParams.append('session_id', activeSessionId);
      const resp = await fetch(url.toString());
      if (!resp.ok) return;
      const data = await resp.json();
      set({ agentRuns: Array.isArray(data) ? data : [] });
    } catch (e) {
      console.error('Error fetching agent runs:', e);
    }
  },

  cancelAgentRun: async (runId) => {
    if (!runId) return;
    set((s) => ({ cancellingRunIds: { ...s.cancellingRunIds, [runId]: true } }));
    try {
      await fetch('/api/agent-cancel', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ run_id: runId }),
      });
      const projectStore = useProjectStore.getState();
      await Promise.all([
        get().fetchAgentRuns(),
        projectStore.fetchAllAgentTrees(),
      ]);
    } catch (e) {
      console.error(`Error cancelling run ${runId}:`, e);
    } finally {
      set((s) => {
        const next = { ...s.cancellingRunIds };
        delete next[runId];
        return { cancellingRunIds: next };
      });
    }
  },

  fetchSessionTokens: async () => {
    const { selectedProjectRoot } = useProjectStore.getState();
    try {
      const resp = await fetch(`/api/status?project_root=${encodeURIComponent(selectedProjectRoot)}`);
      if (resp.ok) {
        const data = await resp.json();
        set({ sessionTokens: { prompt: data.session_prompt_tokens || 0, completion: data.session_completion_tokens || 0 } });
      }
    } catch { /* ignore */ }
  },
}));

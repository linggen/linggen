/**
 * Agents, models, skills, runs, and live activity state.
 */
import { create } from 'zustand';
import type { AgentInfo, AgentRunInfo, AgentTreeItem, ModelInfo, OllamaPsResponse, SkillInfo } from '../types';
import { useSessionStore } from './sessionStore';
import { TOKEN_RATE_WINDOW_MS } from '../lib/messageUtils';
import { dedupFetch } from '../lib/dedupFetch';
import { agentTracker } from '../lib/agentTracker';

export type AgentStatusValue = 'idle' | 'model_loading' | 'thinking' | 'calling_tool' | 'working';

type StateSetter<T> = T | ((prev: T) => T);

interface ServerState {
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
  agentTreesByProject: Record<string, Record<string, AgentTreeItem>>;

  // Agent activity (live status) — all keyed by **session ID**, not agent name.
  agentStatus: Record<string, AgentStatusValue>;       // key: session ID
  agentStatusText: Record<string, string>;              // key: session ID
  agentContext: Record<string, { tokens: number; messages: number; tokenLimit?: number }>; // key: session ID
  tokensPerSec: number; // global — not per-session

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

export const useServerStore = create<ServerState>((set, get) => ({
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
  agentTreesByProject: {},

  agentStatus: {},
  agentStatusText: {},
  agentContext: {},
  tokensPerSec: 0,

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
    agentTracker.recordTokenSample(1);
  },
  recomputeTokenRate: (nowMs) => {
    const rate = agentTracker.pruneAndComputeRate(TOKEN_RATE_WINDOW_MS, nowMs);
    set({ tokensPerSec: rate });
  },

  // Data arrives via page_state push — no HTTP fetch needed
  fetchAgents: async () => {},
  fetchModels: async () => {},

  // Data arrives via page_state push — no HTTP fetch needed
  fetchDefaultModels: async () => {},

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
    // Only poll Ollama when at least one Ollama model is configured
    const hasOllama = get().models.some((m) => m.provider === 'ollama');
    if (!hasOllama) return;
    try {
      const resp = await dedupFetch('/api/utils/ollama-status');
      if (resp.ok) set({ ollamaStatus: await resp.json() });
    } catch (e) {
      console.error('Failed to fetch Ollama status:', e);
    }
  },

  // Data arrives via page_state push — no HTTP fetch needed
  fetchSkills: async () => {},

  reloadSkills: async () => {
    set({ reloadingSkills: true });
    try {
      const { selectedProjectRoot } = useSessionStore.getState();
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
      const { selectedProjectRoot } = useSessionStore.getState();
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

  // Data arrives via page_state push — no HTTP fetch needed
  fetchAgentRuns: async () => {},

  cancelAgentRun: async (runId) => {
    if (!runId) return;
    set((s) => ({ cancellingRunIds: { ...s.cancellingRunIds, [runId]: true } }));
    try {
      await fetch('/api/agent-cancel', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ run_id: runId }),
      });
      await get().fetchAgentRuns();
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
    const { selectedProjectRoot } = useSessionStore.getState();
    try {
      const resp = await dedupFetch(`/api/status?project_root=${encodeURIComponent(selectedProjectRoot)}`);
      if (resp.ok) {
        const data = await resp.json();
        set({ sessionTokens: { prompt: data.session_prompt_tokens || 0, completion: data.session_completion_tokens || 0 } });
      }
    } catch { /* ignore */ }
  },
}));

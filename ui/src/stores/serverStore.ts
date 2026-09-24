/**
 * Agents, models, skills, runs, and live activity state.
 */
import { create } from 'zustand';
import type { AgentInfo, AgentRunInfo, AgentTreeItem, ModelInfo, OllamaPsResponse, RuntimeModelInfo, SkillInfo } from '../types';
import { apiGet } from '../lib/api';
import { useSessionStore } from './sessionStore';
import { TOKEN_RATE_WINDOW_MS } from '../lib/messageUtils';
import { dedupFetch } from '../lib/dedupFetch';
import { agentTracker } from '../lib/agentTracker';
import { agentsApi, appConfig, skillsApi } from '../lib/endpoints';

export type AgentStatusValue = 'idle' | 'model_loading' | 'thinking' | 'calling_tool' | 'working';

type StateSetter<T> = T | ((prev: T) => T);

interface ServerState {
  agents: AgentInfo[];
  models: ModelInfo[];
  /** GET /api/models — the settings screens' view (built-ins, auth mode).
   *  One shared copy; refreshRuntimeModels() re-reads it. */
  runtimeModels: RuntimeModelInfo[];
  refreshRuntimeModels: () => Promise<void>;
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
  // Whether a session is busy is never stored: `isSessionBusy` derives it
  // from pendingSends + agentRuns + the server's busySessions.
  /** page_state.busy_sessions, replaced wholesale on every push: the only
   *  cross-session busy signal (a transport's agentRuns cover its own session). */
  busySessions: Record<string, string>;
  agentStatusText: Record<string, string>;              // key: session ID (display only)
  agentContext: Record<string, { tokens: number; messages: number; tokenLimit?: number }>; // key: session ID
  tokensPerSec: number; // global — not per-session

  /** Optimistic "I just sent a chat" flag, keyed by session ID. Set when
   *  sendChatMessage fires off, cleared on TurnComplete. Bridges the
   *  page_state polling lag so the spinner shows immediately after send
   *  even if `agentRuns` hasn't caught up yet — the user feels instant
   *  feedback, then the real run record takes over. Without this the
   *  spinner is sometimes invisible for fast turns that complete before
   *  the next page_state push. Value = Date.now() when the send fired, so
   *  page_state reconciliation can expire a flag whose TurnComplete was
   *  lost (e.g. session channel closed mid-turn) instead of spinning
   *  forever. Truthiness checks keep working. */
  pendingSends: Record<string, number>;
  setPendingSend: (sessionId: string, pending: boolean) => void;

  /** Timestamp of the last send that never reached the server, keyed by
   *  session ID. Lets the spinner distinguish "turn finished" from "send
   *  failed" when it stops — a failed send has no run to summarize. */
  sendFailedAt: Record<string, number>;
  markSendFailed: (sessionId: string) => void;

  /** Timestamp of a run that vanished server-side without a terminal event
   *  AND without a persisted reply (daemon killed/restarted mid-turn), keyed
   *  by session ID. Consumed once by ChatPanel to flip its run summary from
   *  "{verb} for Xs" — which reads as still working — to "Interrupted". */
  runInterruptedAt: Record<string, number>;
  markRunInterrupted: (sessionId: string) => void;
  consumeRunInterrupted: (sessionId: string) => void;

  // Actions
  setSelectedAgent: (agent: string) => void;
  setBusySessions: (busy: Record<string, string>) => void;
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
  runtimeModels: [],
  refreshRuntimeModels: async () => {
    try {
      const ms = await apiGet<RuntimeModelInfo[]>('/api/models');
      set({ runtimeModels: Array.isArray(ms) ? ms : [] });
    } catch { /* keep the last list */ }
  },
  selectedAgent: typeof window !== 'undefined'
    ? window.localStorage.getItem(SELECTED_AGENT_STORAGE_KEY) || ''
    : '',
  sessionTokens: { prompt: 0, completion: 0 },
  cancellingRunIds: {},
  reloadingSkills: false,
  reloadingAgents: false,
  agentTreesByProject: {},

  busySessions: {},
  agentStatusText: {},
  agentContext: {},
  tokensPerSec: 0,
  pendingSends: {},
  setPendingSend: (sessionId, pending) =>
    set((s) => {
      const next = { ...s.pendingSends };
      if (pending) next[sessionId] = Date.now();
      else delete next[sessionId];
      return { pendingSends: next };
    }),
  sendFailedAt: {},
  markSendFailed: (sessionId) =>
    set((s) => {
      const pending = { ...s.pendingSends };
      delete pending[sessionId];
      return {
        pendingSends: pending,
        sendFailedAt: { ...s.sendFailedAt, [sessionId]: Date.now() },
      };
    }),
  runInterruptedAt: {},
  markRunInterrupted: (sessionId) =>
    set((s) => ({ runInterruptedAt: { ...s.runInterruptedAt, [sessionId]: Date.now() } })),
  consumeRunInterrupted: (sessionId) =>
    set((s) => {
      const next = { ...s.runInterruptedAt };
      delete next[sessionId];
      return { runInterruptedAt: next };
    }),

  setSelectedAgent: (agent) => {
    window.localStorage.setItem(SELECTED_AGENT_STORAGE_KEY, agent);
    set({ selectedAgent: agent });
  },
  setBusySessions: (busy) => set({ busySessions: busy }),
  setAgentStatusText: (updater) => set((s) => ({
    agentStatusText: typeof updater === 'function' ? updater(s.agentStatusText) : updater,
  })),
  setAgentContext: (updater) => set((s) => ({
    agentContext: typeof updater === 'function' ? updater(s.agentContext) : updater,
  })),
  resetStatus: () => set({ busySessions: {}, agentStatusText: {} }),
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
      // Toggle against the file's current value, not the pushed copy.
      const saved = await appConfig.update((config) => {
        const current: string[] = config.routing?.default_models ?? [];
        const next = current.length === 1 && current[0] === modelId ? [] : [modelId];
        return { ...config, routing: { ...config.routing, default_models: next } };
      });
      set({ defaultModels: saved.routing?.default_models ?? [] });
    } catch { /* ignore */ }
  },

  setReasoningEffort: async (modelId, effort) => {
    try {
      let found = false;
      await appConfig.update((config) => {
        const models = [...(config.models ?? [])];
        const idx = models.findIndex((m) => m.id === modelId);
        if (idx === -1) return config;
        found = true;
        models[idx] = { ...models[idx], reasoning_effort: effort || null };
        return { ...config, models };
      });
      if (!found) return;
      set((state) => ({
        models: state.models.map((m) =>
          m.id === modelId ? { ...m, reasoning_effort: effort || null } : m
        ),
      }));
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
    const minSpin = new Promise((r) => setTimeout(r, 1000));
    try {
      const { selectedProjectRoot } = useSessionStore.getState();
      await skillsApi.reload(selectedProjectRoot || undefined);
      await get().fetchSkills();
    } catch (e) {
      console.error('Failed to reload skills:', e);
    } finally {
      await minSpin;
      set({ reloadingSkills: false });
    }
  },

  reloadAgents: async () => {
    set({ reloadingAgents: true });
    try {
      const { selectedProjectRoot } = useSessionStore.getState();
      await agentsApi.reload(selectedProjectRoot || undefined);
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
      await agentsApi.cancelRun(runId);
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

type BusyInputs = Pick<ServerState, 'pendingSends' | 'agentStatusText' | 'agentRuns' | 'busySessions'>;

/** Is this session's agent working? The one answer every surface uses.
 *  - a send in flight (optimistic, until TurnComplete) → busy
 *  - a session this transport has run records for (page_state scopes
 *    agent_runs to the viewed session) → a top-level running run, unless
 *    TurnComplete already said "Idle" (a stale push can re-assert the row);
 *    subagent runs drive the SubagentPane, not the main spinner
 *  - any other session → the server's busy_sessions */
export function isSessionBusy(s: BusyInputs, sid: string | null | undefined): boolean {
  if (!sid) return false;
  if (s.pendingSends[sid]) return true;
  const runs = s.agentRuns.filter((r) => r.session_id === sid);
  if (runs.length > 0) {
    if (s.agentStatusText[sid] === 'Idle') return false;
    return runs.some((r) => !r.parent_run_id && r.status === 'running');
  }
  const busy = s.busySessions[sid];
  return !!busy && busy !== 'idle';
}

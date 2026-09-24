/**
 * Endpoint groups over the typed client (lib/api). Every `/api/*` call the
 * UI makes goes through here or a sibling `*-api.ts`, so the RTC fetch
 * proxy, JSON encoding and error handling are the same everywhere.
 */
import type { AgentFileInfo, AppConfig, BuiltInSkillInfo, ModelHealthInfo, OllamaPsResponse, SkillInfoFull } from '../types';
import { apiDelete, apiGet, apiPatch, apiPost, apiPut } from './api';

const q = (params: Record<string, string | undefined>) => {
  const sp = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) if (v !== undefined) sp.set(k, v);
  return sp.toString();
};

// ── Config ──────────────────────────────────────────────────────────────

export const appConfig = {
  get: () => apiGet<AppConfig>('/api/config'),
  save: (config: AppConfig) => apiPost<void>('/api/config', config),
  /** Read, change, write back. Returns what was written. */
  update: async (change: (config: AppConfig) => AppConfig) => {
    const next = change(await appConfig.get());
    await appConfig.save(next);
    return next;
  },
  setDefaultModels: (ids: string[]) =>
    appConfig.update((c) => ({ ...c, routing: { ...c.routing, default_models: ids } })),
};

// ── Account (linggen.dev) ───────────────────────────────────────────────

export interface Account {
  signed_in?: boolean;
  [key: string]: any;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export const account = {
  get: (app?: string) => apiGet<Account>(`/api/account${app ? `?${q({ app })}` : ''}`),
  logout: () => apiPost<void>('/api/account/logout'),
  /** Start the OS browser login, then wait (≤2 min) for the callback.
   *  The daemon opens the browser itself; a url comes back only when it
   *  couldn't. Resolves the signed-in account, or null on timeout. */
  signIn: async (): Promise<Account | null> => {
    const out = await apiPost<{ opened?: boolean; url?: string }>('/api/account/login').catch(() => null);
    if (out && out.opened === false && out.url) window.open(out.url, '_blank', 'noopener');
    for (let waited = 0; waited < 120_000; waited += 1500) {
      await sleep(1500);
      const acc = await account.get().catch(() => null);
      if (acc?.signed_in) return acc;
    }
    return null;
  },
};

// ── Models, credentials, provider logins ────────────────────────────────

export const modelsApi = {
  health: () => apiGet<ModelHealthInfo[]>('/api/models/health'),
  ollamaStatus: () => apiGet<OllamaPsResponse>('/api/utils/ollama-status'),
  credentials: <T>() => apiGet<T>('/api/credentials'),
  saveCredentials: (body: Record<string, { api_key: string | null; provider: string; url: string }>) =>
    apiPut<void>('/api/credentials', body),
};

/** A provider's browser login (codex, claude): start it, poll its status. */
export const providerAuth = {
  status: <T = { authenticated?: boolean }>(statusUrl: string) => apiGet<T>(statusUrl),
  login: (loginUrl: string) => apiPost<{ opened?: boolean; url?: string }>(loginUrl),
  logout: (logoutUrl: string) => apiPost<void>(logoutUrl),
};

// ── Sessions, chat, workspace ───────────────────────────────────────────

export const sessionApi = {
  setModel: (projectRoot: string, sessionId: string, modelId: string) =>
    apiPatch<void>('/api/sessions', { project_root: projectRoot, session_id: sessionId, model_id: modelId }),
  setPermission: (sessionId: string, path: string, mode: string) =>
    apiPatch<void>('/api/sessions/permission', { session_id: sessionId, path, mode }),
  pendingAskUser: () =>
    apiGet<Array<{ question_id: string; agent_id?: string; questions?: unknown[]; session_id?: string | null }>>('/api/pending-ask-user'),
  clearQueue: (projectRoot: string, sessionId: string, agentId: string) =>
    apiPost<void>('/api/queue/clear', { project_root: projectRoot, session_id: sessionId, agent_id: agentId }),
  systemPrompt: (projectRoot: string, agentId: string, sessionId?: string | null) =>
    apiGet<{ system_prompt?: string; tools?: unknown[] }>(
      `/api/chat/system-prompt?${q({ project_root: projectRoot, agent_id: agentId, session_id: sessionId || undefined })}`,
    ),
  status: (projectRoot: string) => apiGet<any>(`/api/status?${q({ project_root: projectRoot })}`),
  task: (projectRoot: string, agentId: string, task: string) =>
    apiPost<void>('/api/task', { project_root: projectRoot, agent_id: agentId, task }),
};

export interface BashResult { stdout?: string; stderr?: string; output?: string; exit_code?: number }

export const workspaceApi = {
  bash: (projectRoot: string, command: string, sessionId?: string | null) =>
    apiPost<BashResult>('/api/bash', { project_root: projectRoot, command, session_id: sessionId ?? undefined }),
  files: <T>(projectRoot: string, path: string) =>
    apiGet<T[]>(`/api/files?${q({ project_root: projectRoot, path })}`),
  searchFiles: <T>(projectRoot: string, query: string) =>
    apiGet<T[]>(`/api/files/search?${q({ project_root: projectRoot, query })}`),
};

export const userApi = {
  name: () => apiGet<{ name?: string | null }>('/api/user/name'),
};

export const mcpApi = {
  list: <T>() => apiGet<{ servers?: T[] }>('/api/mcp'),
};

// ── Agent + skill files ─────────────────────────────────────────────────

export interface SpecFile { content?: string; valid?: boolean; error?: string; path?: string }

/** The editable markdown specs (agents/*.md, skill files) of a project. */
const specFiles = (listPath: string, filePath: string) => ({
  list: <T = AgentFileInfo>(projectRoot: string) => apiGet<T[]>(`${listPath}?${q({ project_root: projectRoot })}`),
  read: (projectRoot: string, path: string) => apiGet<SpecFile>(`${filePath}?${q({ project_root: projectRoot, path })}`),
  save: (projectRoot: string, path: string, content: string) =>
    apiPost<SpecFile>(filePath, { project_root: projectRoot, path, content }),
  remove: (projectRoot: string, path: string) => apiDelete<void>(filePath, { project_root: projectRoot, path }),
});

export const agentFiles = specFiles('/api/agent-files', '/api/agent-file');
export const skillFiles = specFiles('/api/skill-files', '/api/skill-file');

// ── Skills + marketplace ────────────────────────────────────────────────

export const skillsApi = {
  /** The full list, content included (page_state strips content). */
  list: () => apiGet<SkillInfoFull[]>('/api/skills'),
  reload: (projectRoot?: string) => apiPost<void>('/api/skills/reload', { project_root: projectRoot }),
  builtIn: (refresh = false) => apiGet<BuiltInSkillInfo[]>(`/api/builtin-skills${refresh ? '?refresh=true' : ''}`),
  installBuiltIn: (name: string) => apiPost<void>('/api/builtin-skills/install', { name }),
  searchCommunity: <T>(query: string) => apiGet<T[]>(`/api/community-skills/search?${q({ q: query })}`),
  install: (body: Record<string, unknown>) => apiPost<void>('/api/marketplace/install', body),
  uninstall: (body: Record<string, unknown>) => apiDelete<void>('/api/marketplace/uninstall', body),
  moveToGlobal: (name: string, projectRoot: string) =>
    apiPost<void>('/api/marketplace/move-to-global', { name, project_root: projectRoot }),
};

export const agentsApi = {
  reload: (projectRoot?: string) => apiPost<void>('/api/agents/reload', { project_root: projectRoot }),
  cancelRun: (runId: string) => apiPost<void>('/api/agent-cancel', { run_id: runId }),
};

// ── Phone pairing ───────────────────────────────────────────────────────

export const pairApi = {
  info: <T>() => apiGet<T>('/api/pair/info'),
  qr: <T>(fresh = false) => apiGet<T>(fresh ? '/api/pair/qr?new=true' : '/api/pair/qr'),
  keepalive: () => apiPost<{ generation?: number } | null>('/api/pair/window/keepalive'),
  updateDevice: (id: string, patch: Record<string, unknown>) => apiPatch<void>(`/api/pair/devices/${id}`, patch),
  removeDevice: (id: string) => apiDelete<void>(`/api/pair/devices/${id}`),
};

// ── Storage browser ─────────────────────────────────────────────────────

export const storageApi = {
  roots: <T>() => apiGet<T[]>('/api/storage/roots'),
  tree: <T>(root: string, path: string) => apiGet<{ entries?: T[] }>(`/api/storage/tree?${q({ root, path: path || undefined })}`),
  file: (root: string, path: string) =>
    apiGet<{ content: string; size: number; modified: number }>(`/api/storage/file?${q({ root, path })}`),
  save: (root: string, path: string, content: string) => apiPut<void>('/api/storage/file', { root, path, content }),
  remove: (root: string, path: string) => apiDelete<void>(`/api/storage/file?${q({ root, path })}`),
};

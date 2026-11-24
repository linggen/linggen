/**
 * Endpoint groups over the typed client (lib/api). Every `/api/*` call the
 * UI makes goes through here or a sibling `*-api.ts`, so the RTC fetch
 * proxy, JSON encoding and error handling are the same everywhere.
 */
import type { StatusResponse } from '../types/generated/StatusResponse';
import type { AgentFileInfo, AppConfig, BuiltInSkillInfo, SkillInfoFull } from '../types';
import type { PeopleReply, PersonPatch, PersonSettings } from './people.mts';
import type { OutsideAppsReply } from './outsideApps.mts';
import type { EditionChoice, FamilyAction, ProvidersAnswer } from './providers.mts';
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

};

// ── Account (linggen.dev) ───────────────────────────────────────────────

/** GET /api/account (server/api/account.rs). Signed out → only `signed_in`. */
export interface Account {
  signed_in?: boolean;
  source?: string | null;
  user_name?: string | null;
  avatar_url?: string | null;
  offline?: boolean;
  /** What linggen.dev says this account holds (its own shape; the meter
   *  reads `usage` and `apps`). */
  entitlement?: { usage?: UsageState; apps?: Record<string, unknown>; developer?: boolean } | null;
  /** Present when asked with `?app=`: may this app run. */
  gate?: {
    app?: string | null;
    entitled?: boolean;
    developer?: boolean;
    trial?: TrialState | null;
    allowed?: boolean;
  };
}

/** Monthly allowance on a plan. */
export interface UsageState {
  used: number;
  allowance: number;
  warn: boolean;
  over: boolean;
}

/** A trial's token budget. */
export interface TrialState {
  tokens: number;
  budget: number;
  started_at: number | null;
  expires_at: number | null;
  active: boolean;
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

/** GET /api/models/signed-in-default: the model a sign-in just made the
 *  default, the one line that says so, and the chain to adopt; null when
 *  the sign-in changed nothing. */
export interface SignedInDefault {
  model: string;
  line: string;
  default_models: string[];
}

export const modelsApi = {
  signedInDefault: () => apiGet<SignedInDefault | null>('/api/models/signed-in-default'),
  /** Stay on a ChatGPT family's previous model, or follow it again. */
  stayChatgpt: (family: string, stay: boolean) =>
    apiPost<void>('/api/models/chatgpt/stay', { family, stay }),
};

/** The owner's provider settings (src/server/api/providers.rs and
 *  provider_families.rs). Every call answers with the fresh list; the page
 *  draws that answer and decides nothing itself (lib/providers.mts). */
const provider = (id: string) => `/api/providers/${encodeURIComponent(id)}`;
export const usesApi = {
  list: () => apiGet<import('./modelUses').Uses>('/api/uses'),
  put: (edit: import('./modelUses').UseEdit) => apiPut<import('./modelUses').Uses>('/api/uses', edit),
};

export const providersApi = {
  list: () => apiGet<ProvidersAnswer>('/api/providers'),
  /** A provider the person picked from `presets`, by its label. */
  create: (preset: string) => apiPost<ProvidersAnswer>('/api/providers', { preset }),
  /** The owner's choice of edition: what is offered for new setups only. */
  setEdition: (choice: EditionChoice) => apiPut<ProvidersAnswer>('/api/providers/edition', { choice }),
  setKey: (id: string, apiKey: string | null) => apiPut<ProvidersAnswer>(`${provider(id)}/key`, { api_key: apiKey }),
  keepModelKey: (id: string) => apiPut<ProvidersAnswer>(`${provider(id)}/key`, { action: 'keep_model_key' }),
  setAddress: (id: string, url: string) => apiPatch<ProvidersAnswer>(provider(id), { url }),
  /** A model typed by name; the engine makes the row's id. */
  addModel: (id: string, model: string) => apiPost<ProvidersAnswer>(`${provider(id)}/models`, { model }),
  removeModel: (id: string, modelId: string) => apiDelete<ProvidersAnswer>(`${provider(id)}/models/${encodeURIComponent(modelId)}`),
  /** Tick, untick, choose, pin, follow, or answer a notice (`move` is its number). */
  family: (id: string, family: string, action: FamilyAction) =>
    apiPut<ProvidersAnswer>(`${provider(id)}/families/${encodeURIComponent(family)}`, action),
};

/** A provider's browser login (codex, claude): start it, poll its status. */
export const providerAuth = {
  status: <T = { authenticated?: boolean }>(statusUrl: string) => apiGet<T>(statusUrl),
  login: (loginUrl: string) => apiPost<{ opened?: boolean; url?: string }>(loginUrl),
  logout: (logoutUrl: string) => apiPost<void>(logoutUrl),
};

// ── Sessions, chat, workspace ───────────────────────────────────────────

export const sessionApi = {
  /** Set `agentId`'s model in the session (default: the member the Mac
   *  answers with by default). */
  setModel: (projectRoot: string, sessionId: string, modelId: string, agentId?: string) =>
    apiPatch<void>('/api/sessions', { project_root: projectRoot, session_id: sessionId, model_id: modelId, ...(agentId ? { agent_id: agentId } : {}) }),
  /** Let (or stop) a failed model call be answered by another model in the session. */
  setModelFallback: (sessionId: string, allowed: boolean) =>
    apiPatch<void>('/api/sessions', { session_id: sessionId, model_fallback: allowed }),
  /** The conversation's thinking effort (`''`: back to the chat default). `agentId`: the member whose model must offer it. */
  setEffort: (sessionId: string, effort: string, agentId?: string) =>
    apiPatch<void>('/api/sessions', { session_id: sessionId, effort, ...(agentId ? { agent_id: agentId } : {}) }),
  setWork: (sessionId: string, work: 'code' | 'none') =>
    apiPatch<void>('/api/sessions', { session_id: sessionId, work }),
  setPermission: (sessionId: string, path: string, mode: string) =>
    apiPatch<void>('/api/sessions/permission', { session_id: sessionId, path, mode }),
  pendingAskUser: () =>
    apiGet<Array<import('./askQueue.mts').ListedAsk & { session_id?: string | null }>>('/api/pending-ask-user'),
  /** Drop one queued message; it comes back (text + pictures) for an Edit. */
  removeQueued: (sessionId: string, agentId: string, id: string) =>
    apiPost<{ message?: string; images?: string[] }>('/api/queue/remove', { session_id: sessionId, agent_id: agentId, id }),
  /** Stop one background task of the session; never the turn or the others. */
  stopTask: (sessionId: string, id: string) => apiPost<void>('/api/tasks/stop', { session_id: sessionId, id }),
  /** Send one queued message into the running turn now. */
  steerQueued: (id: string) => apiPost<void>('/api/queue/steer', { id }),
  /** `skillName`: the app whose page asks — the export shows that app's place. */
  systemPrompt: (projectRoot: string, agentId: string, sessionId?: string | null, skillName?: string) =>
    apiGet<{ system_prompt?: string; tools?: unknown[] }>(
      `/api/chat/system-prompt?${q({ project_root: projectRoot, agent_id: agentId, session_id: sessionId || undefined, skill_name: skillName })}`,
    ),
  status: (projectRoot: string) => apiGet<StatusResponse>(`/api/status?${q({ project_root: projectRoot })}`),
  task: (projectRoot: string, agentId: string, task: string) =>
    apiPost<void>('/api/task', { project_root: projectRoot, agent_id: agentId, task }),
};

export interface BashResult { stdout?: string; stderr?: string; output?: string; exit_code?: number }

export const workspaceApi = {
  /** `skillName`: the app whose page sends it — the command stands at that app's place. */
  bash: (projectRoot: string, command: string, sessionId?: string | null, skillName?: string) =>
    apiPost<BashResult>('/api/bash', { project_root: projectRoot, command, session_id: sessionId ?? undefined, skill_name: skillName }),
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

// ── People (Settings → People) ──────────────────────────────────────────

/** The household as this Mac's owner sets it (server/api/people.rs): each person's
 *  level, apps, models and daily limit. A person is the string the engine keeps them as. */
export const peopleApi = {
  list: () => apiGet<PeopleReply>('/api/people'),
  set: (person: string, patch: PersonPatch) =>
    apiPut<{ settings: PersonSettings }>(`/api/people/${encodeURIComponent(person)}`, patch),
  /** The apps no one else uses. */
  keepForOwner: (apps: string[]) => apiPut<{ owner: PeopleReply['owner'] }>('/api/people/owner', { mine_only: apps }),
};

/** The private apps agents outside Linggen may use (server/api/outside_apps.rs). */
export const outsideAppsApi = {
  list: () => apiGet<OutsideAppsReply>('/api/mcp/outside-apps'),
  /** Exactly these apps are open now. */
  set: (apps: string[]) => apiPut<{ apps: string[] }>('/api/mcp/outside-apps', { apps }),
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

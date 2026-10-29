/**
 * Typed API client.
 *
 * Thin wrapper over `fetch()` for `/api/*` endpoints. Goes through the
 * existing `fetchProxy` (which routes via WebRTC when connected) — this
 * file only adds typing, JSON encoding, and consistent error handling.
 *
 * Endpoint helpers (`sessions.*`, etc.) live below the generic core.
 */
import type { RunningWork } from './runningWork.mts';

export class ApiError extends Error {
  constructor(public readonly status: number, message: string) {
    super(message);
    this.name = 'ApiError';
  }
}

/** A user-facing message for a failed call: the server's `{ "error": … }`
 *  when it sent one, else `fallback` (or the raw text). */
export function apiErrorMessage(e: unknown, fallback?: string): string {
  if (e instanceof ApiError) {
    try {
      const body = JSON.parse(e.message);
      if (body && typeof body.error === 'string') return body.error;
    } catch { /* not JSON */ }
    return fallback ?? e.message;
  }
  return e instanceof Error ? e.message : fallback ?? String(e);
}

async function parseError(resp: Response): Promise<ApiError> {
  const text = await resp.text().catch(() => '');
  return new ApiError(resp.status, text || `HTTP ${resp.status}`);
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const resp = await fetch(path, init);
  if (!resp.ok) throw await parseError(resp);
  if (resp.status === 204) return undefined as T;
  const ct = resp.headers.get('content-type') || '';
  if (ct.includes('application/json')) return (await resp.json()) as T;
  return (await resp.text()) as unknown as T;
}

const jsonHeaders = { 'Content-Type': 'application/json' };

export const apiGet = <T>(path: string): Promise<T> =>
  request<T>(path);

export const apiPost = <T>(path: string, body?: unknown): Promise<T> =>
  request<T>(path, {
    method: 'POST',
    headers: jsonHeaders,
    body: body === undefined ? undefined : JSON.stringify(body),
  });

export const apiPatch = <T>(path: string, body?: unknown): Promise<T> =>
  request<T>(path, {
    method: 'PATCH',
    headers: jsonHeaders,
    body: body === undefined ? undefined : JSON.stringify(body),
  });

export const apiPut = <T>(path: string, body?: unknown): Promise<T> =>
  request<T>(path, {
    method: 'PUT',
    headers: jsonHeaders,
    body: body === undefined ? undefined : JSON.stringify(body),
  });

export const apiDelete = <T>(path: string, body?: unknown): Promise<T> =>
  request<T>(path, {
    method: 'DELETE',
    headers: jsonHeaders,
    body: body === undefined ? undefined : JSON.stringify(body),
  });

// ---------------------------------------------------------------------------
// Endpoint helpers
// ---------------------------------------------------------------------------

interface SessionCreateRequest {
  runtime?: 'claude_agent_sdk';
  title: string;
  /** A skill-bound session (an app page's chat): binds the skill's rules and tools. */
  skill?: string;
  /** The values a skill session is bound to (its declared session params). */
  params?: Record<string, string>;
}
interface SessionCreateResponse {
  runtime?: 'claude_agent_sdk';
  agents?: import('./sessionMembers.mts').SessionMember[];
  id: string;
  /** The folder the engine started the session in. */
  cwd?: string;
}
interface SessionDeleteRequest {
  session_id: string;
  project: string | null;
  mission_id: string | null;
  skill: string | null;
}
interface SessionRenameRequest {
  session_id: string;
  title: string;
}

export const sessions = {
  create: (req: SessionCreateRequest) =>
    apiPost<SessionCreateResponse>('/api/sessions', req),
  remove: (req: SessionDeleteRequest) =>
    apiDelete<void>('/api/sessions/all', req),
  rename: (req: SessionRenameRequest) =>
    apiPatch<void>('/api/sessions', req),
  /** What deleting the session would stop. */
  running: (sessionId: string) =>
    apiGet<RunningWork>(`/api/sessions/running?session_id=${encodeURIComponent(sessionId)}`),
};

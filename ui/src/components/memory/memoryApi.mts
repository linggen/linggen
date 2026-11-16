/**
 * The Memory tab's one door: `POST /api/memory/<verb>` with a JSON body,
 * carried over the data channel like every other `/api/*` call (the engine
 * applies the per-caller access rule). Nothing else in the UI names this path.
 *
 * Kept free of React and of `lib/api.ts` so node can load it under test.
 */

export type Tier = 'core' | 'semantic' | 'episodic';
/** The browser's tier tabs: everything, or one tier. */
export type TabKey = 'all' | Tier;

export const TYPES = ['fact', 'preference', 'decision', 'tried', 'fixed', 'learned', 'built'] as const;
export const ORIGINS = ['user', 'agent', 'derived'] as const;
export const OUTCOMES = ['positive', 'negative', 'neutral'] as const;

/** A row as the door returns it — the embedding vector is never kept. */
export interface MemoryRow {
  id: string;
  content: string;
  type: string;
  tier: Tier;
  from: string;
  outcome?: string | null;
  scope?: string | null;
  summary?: string | null;
  indexed?: boolean;
  created_at?: string;
  updated_at?: string | null;
  occurred_at?: string | null;
  /** Search only: 0..1 relevance. */
  hybrid_score?: number;
  [field: string]: unknown;
}

export interface ReviewIssue {
  id: string; kind: string; note: string; row_ids: string[]; created_at: string; status: string;
}
export interface ReviewQueue { open_count: number; issues: ReviewIssue[] }

export interface MemoryDay {
  date: string; state?: string; dreamed?: boolean; scanned?: boolean; rows: number; unjudged: number; past_ttl: number;
  harvested_at: string | null; remembered_at: string | null; judged: number; promoted: number | null; forgotten: number | null;
}
export interface MemoryDays {
  today: string; ttl_days: number; first_unscanned: string | null; first_undreamed: string | null; days: MemoryDay[];
}

export interface MemoryStats {
  total: number;
  per_tier: Record<Tier, number>;
  expired: number;
  disk_bytes: { total: number };
  last_remembered_at: string | null;
}

/** What narrows a read or a forget. Dates are `YYYY-MM-DD`. */
export interface Filter {
  type?: string;
  from?: string;
  until?: string;
}

export type Sort = 'newest' | 'oldest';
export const PAGE = 50;

// ── Errors ──────────────────────────────────────────────────────────────

/** The door answered with a refusal, or could not be reached. */
export class MemoryError extends Error {
  status: number;
  code: string;
  constructor(message: string, status: number, code: string) {
    super(message);
    this.name = 'MemoryError';
    this.status = status;
    this.code = code;
  }
  /** This viewer is not allowed memory. */
  get forbidden(): boolean {
    return this.status === 403;
  }
}

/** One plain line (≤ 20 words) for a failed call, `doing` as "read memory". */
export function describeError(e: unknown, doing: string): string {
  if (e instanceof MemoryError && e.forbidden) return 'This device is not allowed to open memory.';
  if (e instanceof MemoryError && e.status === 502) return 'Memory is not reachable right now.';
  const why = e instanceof Error ? e.message : String(e);
  return `Could not ${doing}: ${why}`.slice(0, 160);
}

// ── Transport ───────────────────────────────────────────────────────────

/** `v` without any embedding vector, at any depth (rows come inside
 *  `{action, fact}` and lists). */
export function withoutVectors<T>(v: T): T {
  if (Array.isArray(v)) return v.map(withoutVectors) as T;
  if (v === null || typeof v !== 'object') return v;
  const out: Record<string, unknown> = {};
  for (const [k, val] of Object.entries(v)) if (k !== 'vector') out[k] = withoutVectors(val);
  return out as T;
}

async function call<T>(verb: string, body: Record<string, unknown> = {}): Promise<T> {
  const res = await fetch(`/api/memory/${verb}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  const envelope = await res.json().catch(() => null);
  if (!res.ok || !envelope?.ok) {
    throw new MemoryError(envelope?.error ?? `HTTP ${res.status}`, res.status, envelope?.code ?? 'NETWORK');
  }
  return withoutVectors(envelope.data) as T;
}

// ── Request shapes ──────────────────────────────────────────────────────

/** Which table(s) a tab reads: `all` spans both; the others pin one (the
 *  semantic table holds core and long-term, told apart by `tier`). */
export function tabScope(tab: TabKey): Record<string, unknown> {
  if (tab === 'all') return {};
  if (tab === 'episodic') return { episodic: true };
  return { tier: tab, episodic: false };
}

function filterBody(f: Filter): Record<string, unknown> {
  const body: Record<string, unknown> = {};
  if (f.type) body.type = f.type;
  if (f.from) body.from = f.from;
  if (f.until) body.until = f.until;
  return body;
}

export function listBody(tab: TabKey, f: Filter, sort: Sort, offset: number): Record<string, unknown> {
  return { ...tabScope(tab), ...filterBody(f), sort, limit: PAGE, offset };
}

/** Search for people: every match ranked, never an empty list for a terse
 *  query (`min_score: 0` lifts the recall floor meant for the agents). */
export function searchBody(text: string, tab: TabKey, f: Filter): Record<string, unknown> {
  return { query: text, ...tabScope(tab), ...filterBody(f), limit: PAGE, min_score: 0 };
}

/** Where a forget or count looks: one table, with the tier that picks core or
 *  long-term inside the semantic table. */
export type Where = 'core' | 'semantic' | 'episodic';
/** `all` is every tier: each is asked on its own, because count and forget
 *  default to different tables when none is named. */
export type Reach = Where | 'all';

export function reachWhere(reach: Reach): Where[] {
  return reach === 'all' ? ['core', 'semantic', 'episodic'] : [reach];
}

/** Does a forget carry a filter the person chose (never just a tier)? */
export function isNarrowed(f: Filter): boolean {
  return Boolean(f.type || f.from || f.until);
}

export function whereBody(where: Where, f: Filter): Record<string, unknown> {
  return { ...tabScope(where), ...filterBody(f) };
}

/** What a new row is made of. */
export interface NewRow {
  content: string;
  tier: Tier;
  type: string;
  from: string;
  outcome?: string;
  summary?: string;
  indexed?: boolean;
  /** A directory the row is about; empty = about the person. */
  scope?: string;
}

export function addBody(row: NewRow): Record<string, unknown> {
  const body: Record<string, unknown> = {
    content: row.content, type: row.type, from: row.from,
  };
  // An add with no tier would land episodic: always name it.
  if (row.tier === 'episodic') body.episodic = true;
  else body.tier = row.tier;
  if (row.outcome) body.outcome = row.outcome;
  if (row.summary) body.summary = row.summary;
  if (row.indexed && row.tier !== 'core') body.indexed = true;
  if (row.scope && row.tier !== 'core') body.scope = row.scope;
  else body.global = true;
  return body;
}

/** The fields an edit can change; `null` clears a nullable one. */
export interface Patch {
  content?: string;
  summary?: string | null;
  indexed?: boolean;
  type?: string;
  from?: string;
  tier?: Tier;
  outcome?: string | null;
  scope?: string | null;
}

export function updateBody(id: string, patch: Patch): Record<string, unknown> {
  // This page is the person's own hands: an edit here is directed by them,
  // which the store's guard on rewriting a user-voiced row asks to be told.
  const body: Record<string, unknown> = { id, user_directed: true };
  for (const key of ['content', 'indexed', 'type', 'from', 'tier'] as const) {
    if (patch[key] !== undefined) body[key] = patch[key];
  }
  for (const key of ['summary', 'outcome', 'scope'] as const) {
    if (patch[key] === undefined) continue;
    if (patch[key] === null) body[`clear_${key}`] = true;
    else body[key] = patch[key];
  }
  return body;
}

// ── The verbs ───────────────────────────────────────────────────────────

export const memory = {
  stats: () => call<MemoryStats>('stats'),
  days: () => call<MemoryDays>('days', {}),
  issues: () => call<ReviewQueue>('issues', { status: 'open', limit: 1000 }),
  get: (id: string) => call<MemoryRow | null>('get', { id }),
  dismiss: (id: string) => call<unknown>('issue_resolve', { id, outcome: 'dismissed', note: 'dismissed from the browse page' }),

  list: (tab: TabKey, f: Filter, sort: Sort, offset: number) =>
    call<MemoryRow[]>('list', listBody(tab, f, sort, offset)),

  search: (text: string, tab: TabKey, f: Filter) =>
    call<MemoryRow[]>('search', searchBody(text, tab, f)),

  /** How many rows a filter reaches. */
  count: async (reach: Reach, f: Filter) => {
    const counts = await Promise.all(reachWhere(reach).map(async (w) =>
      (await call<{ count: number }>('count', whereBody(w, f))).count));
    return counts.reduce((a, b) => a + b, 0);
  },

  /** `merged` means a duplicate was found and this is the row that kept it. */
  add: async (row: NewRow) => {
    const out = await call<{ action: string; fact: MemoryRow }>('add', addBody(row));
    return { merged: out.action === 'merged', row: out.fact };
  },

  update: (id: string, patch: Patch) => call<MemoryRow>('update', updateBody(id, patch)),

  /** Whether a row was removed: a missing id answers `removed: false`. */
  remove: async (id: string) => (await call<{ removed: boolean }>('delete', { id })).removed,

  /** Removes what a filter reaches. The server refuses an empty filter, and
   *  so does this: a forget needs a type, a source or a date. */
  forget: async (reach: Reach, f: Filter) => {
    if (!isNarrowed(f)) throw new MemoryError('Pick a type, a source or a date first.', 400, 'BAD_REQUEST');
    let removed = 0;
    for (const w of reachWhere(reach)) {
      removed += (await call<{ removed: number }>('forget', whereBody(w, f))).removed;
    }
    return removed;
  },
};

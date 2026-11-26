/**
 * Memory's two own settings, through the engine's `/api/ling-mem/config`
 * (carried over the data channel like every other `/api/*` call; the engine
 * lets only the Mac's owner through). Not `/api/memory/<verb>` — that door
 * is `memoryApi.mts`.
 *
 * Kept free of React so node can load it under test.
 */
import { MemoryError } from './memoryApi.mts';

export interface MemorySettings {
  /** How many days a short-term row is kept. */
  episodic_ttl_days: number;
  /** The lowest match recall may bring back, 0..1. */
  recall_min_score: number;
}

export const TTL_MIN = 1;
export const TTL_MAX = 365;

const PATH = '/api/ling-mem/config';

async function call(method: 'GET' | 'PUT', body?: MemorySettings): Promise<MemorySettings> {
  const res = await fetch(PATH, {
    method,
    ...(body && { headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) }),
  });
  const envelope = await res.json().catch(() => null);
  if (!res.ok || !envelope?.ok) {
    throw new MemoryError(envelope?.error ?? `HTTP ${res.status}`, res.status, envelope?.code ?? 'NETWORK');
  }
  return envelope.data as MemorySettings;
}

export const readSettings = () => call('GET');
export const saveSettings = (settings: MemorySettings) => call('PUT', settings);

/** What the two fields hold as typed. */
export interface Draft { days: string; score: string }

export const draftOf = (s: MemorySettings): Draft => ({ days: String(s.episodic_ttl_days), score: String(s.recall_min_score) });

/** The typed values as settings, or one plain line saying what is wrong. */
export function parseDraft(d: Draft): { settings: MemorySettings } | { error: string } {
  const days = Number(d.days.trim());
  if (!d.days.trim() || !Number.isInteger(days) || days < TTL_MIN || days > TTL_MAX) {
    return { error: `Days must be a whole number from ${TTL_MIN} to ${TTL_MAX}.` };
  }
  const score = Number(d.score.trim());
  if (!d.score.trim() || !Number.isFinite(score) || score < 0 || score > 1) {
    return { error: 'The threshold must be a number from 0 to 1.' };
  }
  return { settings: { episodic_ttl_days: days, recall_min_score: score } };
}

/** Has the person changed either field from what is stored? */
export function isChanged(d: Draft, stored: MemorySettings): boolean {
  const was = draftOf(stored);
  return d.days.trim() !== was.days || d.score.trim() !== was.score;
}

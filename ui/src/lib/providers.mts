/**
 * What `GET /api/providers` answers, and the reading of it Settings → Models
 * needs. The engine decides everything here — which families a key has, what
 * each runs now, its price, which row moved; this file only picks parts of the
 * answer to draw and gives a route's failure its fixed label. It holds no
 * preset map, no winner rule and no price rule. Pure, so node tests can hold it.
 */
import { W, type Word } from './modelsWords.mts';

export type Way = 'key' | 'local' | 'account' | 'subscription';
export type Kind = 'chat' | 'live_voice' | 'speak' | 'hear' | 'draw';

export interface ProviderModel {
  id: string;
  model: string;
  kind: Kind;
  ready: boolean;
  why?: string;
  builtin: boolean;
  first?: boolean;
}

/** USD per million tokens, at one date. */
export interface PriceLine {
  input: number;
  output: number;
  cache_read: number;
  cache_write: number;
  source: 'catalog' | 'config';
  as_of?: string;
}

export interface ModelLine {
  model: string;
  name: string;
  released?: string;
  price?: PriceLine;
}

export interface Choice {
  model: string;
  name: string;
  released?: string;
  held?: string;
  chooseable: boolean;
}

export interface Notice {
  move: number;
  kind: 'followed' | 'went_back';
  from: string;
  to: string;
  on: string;
  can_go_back: boolean;
}

export interface Reference {
  place: 'default_chat' | 'fallback' | 'pet' | 'agent';
  who?: string;
}

export interface Family {
  family: string;
  label: string;
  ticked: boolean;
  row?: string;
  mode?: 'follow' | 'pinned';
  current?: ModelLine;
  next?: ModelLine;
  hold?: string;
  choose: Choice[];
  notice?: Notice;
  folded: boolean;
  references: Reference[];
  merge_into?: string;
}

export interface FamiliesView {
  access?: 'checking' | 'list_failed';
  rates_unavailable?: boolean;
  catalog: { observed_at: string; age_days: number; stale: boolean };
  families: Family[];
}

export interface Provider {
  id: string;
  label: string;
  way: Way;
  provider: string;
  url: string;
  connected: boolean;
  fix?: 'add_key' | 'sign_in' | 'download' | 'install';
  key?: { set: boolean; conflict?: boolean; can_keep_model_key?: boolean };
  models: ProviderModel[];
  used_by: string[];
  disk_gb?: number;
  families?: FamiliesView;
}

export interface Preset {
  name: string;
  label: string;
  url: string;
  way: Way;
}

export interface ProvidersAnswer {
  presets: Preset[];
  providers: Provider[];
}

/** What a family action sends (PUT /api/providers/{id}/families/{family}). */
export type FamilyAction =
  | { action: 'tick' }
  | { action: 'merge'; model: string }
  | { action: 'untick'; model?: string }
  | { action: 'choose'; model: string }
  | { action: 'follow' }
  | { action: 'pin'; move?: number }
  | { action: 'ack'; move: number }
  | { action: 'back'; move: number };

// ── The two groups of the provider list ──

/** 开箱即用: a sign-in, no key. */
export const readyToUse = (providers: Provider[]): Provider[] =>
  providers.filter((p) => p.way === 'account' || p.way === 'subscription');

/** 自己的密钥: the key providers, in the order the engine lists them. */
export const keyProviders = (providers: Provider[]): Provider[] => providers.filter((p) => p.way === 'key');

/** This Mac's lanes and servers, which the page shows as one group. */
export const localProviders = (providers: Provider[]): Provider[] => providers.filter((p) => p.way === 'local');

// ── What a key row draws ──

/** The family rows shown at once, and the ones folded under 「更多」. */
export function splitFamilies(view: FamiliesView | undefined): { shown: Family[]; folded: Family[] } {
  const all = view?.families ?? [];
  return { shown: all.filter((f) => !f.folded), folded: all.filter((f) => f.folded) };
}

/** The folded families that match a search (a name, or the family's key). */
export function searchFamilies(list: Family[], query: string): Family[] {
  const q = query.trim().toLowerCase();
  if (!q) return list;
  return list.filter((f) => [f.label, f.family, f.current?.name, f.next?.name].some((s) => s?.toLowerCase().includes(q)));
}

/** The provider's chat rows no family stands behind (a model added by name). */
export function addedByName(provider: Provider): ProviderModel[] {
  const rows = new Set((provider.families?.families ?? []).map((f) => f.row));
  return provider.models.filter((m) => m.kind === 'chat' && !rows.has(m.id));
}

/** The live-voice lines of a provider: tagged, never a chat default. */
export const liveModels = (provider: Provider): ProviderModel[] => provider.models.filter((m) => m.kind === 'live_voice');

/** What a family would run (or runs): the engine's own line, current first. */
export const familyModel = (f: Family): ModelLine | undefined => f.current ?? f.next;

/** Where a ticked family is used, as the words for it. */
export function usedWords(r: Reference): { word: Word; who?: string } {
  switch (r.place) {
    case 'default_chat': return { word: W.usedDefault };
    case 'fallback': return { word: W.usedFallback };
    case 'pet': return { word: W.usedCompanion };
    case 'agent': return { word: W.usedAgent, who: r.who };
  }
}

/** USD as the engine gave it, no trailing zeros: 4 → "4", 0.1 → "0.1". */
export const dollars = (n: number): string => String(Number(n.toFixed(4)));

// ── The default model card ──

/** A model a new chat can start on: ready, and a chat model. */
export interface ChatChoice {
  id: string;
  group: string;
  text: string;
}

/** The ready chat models, by provider; a family's row is named by what it runs. */
export function chatChoices(providers: Provider[]): ChatChoice[] {
  const out: ChatChoice[] = [];
  for (const p of providers) {
    const names = new Map((p.families?.families ?? []).flatMap((f) => (f.row && f.current ? [[f.row, f.current.name] as const] : [])));
    for (const m of p.models) {
      if (m.kind === 'chat' && m.ready) out.push({ id: m.id, group: p.label, text: names.get(m.id) ?? m.model });
    }
  }
  return out;
}

const USES: { kind: Exclude<Kind, 'chat'>; word: Word }[] = [
  { kind: 'live_voice', word: W.liveVoice },
  { kind: 'speak', word: W.readAloud },
  { kind: 'hear', word: W.dictation },
  { kind: 'draw', word: W.drawing },
];

/** The other uses as facts: which model each has and whether it is ready. */
export interface UseLine {
  word: Word;
  model: string;
  ready: boolean;
}

export function useLines(providers: Provider[]): UseLine[] {
  return USES.flatMap(({ kind, word }) =>
    providers.flatMap((p) => p.models.filter((m) => m.kind === kind).map((m) => ({ word, model: m.model, ready: m.ready }))),
  );
}

// ── A route's failure, as a fixed label ──

/** The fixed answers of the provider routes (src/server/api/providers.rs,
 *  provider_families.rs), each with the label that says it. */
const FAILURES: [string, Word][] = [
  ['Invalid provider settings.', W.failNotAccepted],
  ['unsupported', W.failNotAccepted],
  ['Provider or model not found.', W.failNotFound],
  ['missing', W.failNotFound],
  ['Could not save provider settings.', W.failNotSaved],
  ['That provider address or model ID is already in use.', W.failInUse],
  ['duplicate_model_id', W.failInUse],
  ['first_default', W.failFirstDefault],
  ['no_key', W.failNoKey],
  ['needs_choice', W.failNeedsChoice],
  ['no_access', W.failNoAccess],
  ['no_previous', W.failNoPrevious],
];

/** The label for a failed call: `status` and the body text it came back with. */
export function failureLabel(status: number | undefined, body: string): Word {
  if (status === 403) return W.failOwnerOnly;
  const text = body.trim();
  return FAILURES.find(([answer]) => answer === text)?.[1] ?? W.failOther;
}

/** Two addresses that name the same place (a trailing slash, capitals aside). */
export const sameAddress = (a: string, b: string): boolean =>
  a.trim().replace(/\/+$/, '').toLowerCase() === b.trim().replace(/\/+$/, '').toLowerCase();

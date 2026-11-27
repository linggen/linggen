/**
 * What `GET /api/providers` answers, and the reading of it Settings → Models
 * needs. The engine decides everything here; this file only picks parts of
 * the answer to draw and gives a route's failure its fixed label. No model
 * list, no price. Pure, so node tests can hold it.
 */
import type { Word } from './modelsWords.mts';
import { W } from './modelsWords.mts';

export type Way = 'key' | 'local' | 'account' | 'subscription';
export type Kind = 'chat' | 'live_voice' | 'speak' | 'hear' | 'draw';
export type ApiStyle = 'openai' | 'claude';

export interface ProviderModel {
  id: string;
  model: string;
  /** The provider's own name, else the id. */
  name: string;
  kind: Kind;
  ready: boolean;
  why?: string;
  builtin: boolean;
  first?: boolean;
}

/** One of Linggen Cloud's models: a checkbox, and the row that runs it when ticked. */
export interface CloudLine {
  model: string;
  label: string;
  row?: string;
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
  /** A preset owns the address: no address field. */
  listed?: boolean;
  style?: ApiStyle;
  /** Linggen Cloud's models to tick. */
  cloud?: CloudLine[];
  /** The ChatGPT sign-in's card: its models are the ticked families and the exact versions added. */
  chatgpt?: boolean;
}

export interface Preset {
  name: string;
  label: string;
  url: string;
  way: Way;
  style: ApiStyle;
}

export interface ProvidersAnswer {
  presets: Preset[];
  providers: Provider[];
}

/** A model id a key provider lists, suggested under the model input. */
export interface Suggestion {
  id: string;
  name?: string;
}

// ── The two groups of the provider list ──

/** The fixed top block: Linggen Cloud, the one account that is always there. */
export const readyToUse = (providers: Provider[]): Provider[] => providers.filter((p) => p.way === 'account');

/** 自己的服务商: the key providers, Ollama (an address, no key) and the ChatGPT sign-in once added, in the order the engine lists them. */
export const ownProviders = (providers: Provider[]): Provider[] => providers.filter((p) => p.way === 'key' || p.way === 'local' || p.way === 'subscription');

/** The ChatGPT sign-in's row: a subscription provider with models to tick. */
export const isChatgpt = (p: Provider): boolean => p.way === 'subscription' && !!p.chatgpt;

/** Ollama: a provider with an address and no key. */
export const isOllama = (p: Provider): boolean => p.way === 'local';

/** A provider the person must look at: a key clash, or a key set but not working. */
export const hasProblem = (p: Provider): boolean =>
  p.way === 'key' && (!!p.key?.conflict || (!!p.key?.set && !p.connected));

// ── What a key row draws ──

/** The provider's chat models, as the person added them. */
export const chatModels = (provider: Provider): ProviderModel[] => provider.models.filter((m) => m.kind === 'chat');

/** The live-voice lines of a provider: tagged, never a chat default. */
export const liveModels = (provider: Provider): ProviderModel[] => provider.models.filter((m) => m.kind === 'live_voice');

/** The suggestions that match what is typed, leaving out what is added: no id is hidden otherwise. */
export function suggestionsFor(list: Suggestion[] | null | undefined, typed: string, added: string[], max = 8): Suggestion[] {
  const q = typed.trim().toLowerCase();
  if (!list || !q) return [];
  const have = new Set(added);
  return list.filter((s) => !have.has(s.id) && (s.id.toLowerCase().includes(q) || !!s.name?.toLowerCase().includes(q))).slice(0, max);
}

/** Where a provider's models are used in the default chain: what the remove confirm names. */
export function heldDefaults(provider: Provider, chain: string[]): { first: boolean; fallbacks: string[] } {
  const mine = new Set(provider.models.map((m) => m.id));
  return { first: !!chain[0] && mine.has(chain[0]), fallbacks: chain.slice(1).filter((id) => mine.has(id)) };
}

// ── The default model card ──

/** A model a new chat can start on: ready, and a chat model. */
export interface ChatChoice {
  id: string;
  group: string;
  text: string;
}

/** The ready chat models, by provider, by the name the engine gives each. */
export function chatChoices(providers: Provider[]): ChatChoice[] {
  return providers.flatMap((p) => p.models.filter((m) => m.kind === 'chat' && m.ready).map((m) => ({ id: m.id, group: p.label, text: m.name || m.model })));
}

// ── A route's failure, as a fixed label ──

/** The fixed answers of the provider routes (src/server/api/providers.rs),
 *  each with the label that says it. */
const FAILURES: [string, Word][] = [
  ['Invalid provider settings.', W.failNotAccepted],
  ['Provider or model not found.', W.failNotFound],
  ['Could not save provider settings.', W.failNotSaved],
  ['That provider address or model ID is already in use.', W.failInUse],
  ['duplicate_model_id', W.failInUse],
  ['first_default', W.failFirstDefault],
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

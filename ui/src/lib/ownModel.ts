/**
 * The second way on when the Linggen Cloud allowance is spent: the person's
 * own model (a ChatGPT or Claude sign-in, or an API key) — Hanli 2026-11-03,
 * 「试用期到期后, 要有消息提示用户订阅或者添加自己的模型」.
 *
 * `openModelsSettings` opens Settings → Models from wherever the chat runs:
 * the launcher's settings modal, the dev console's /settings, or — from a
 * page no host frames — a new tab. `waitForOwnModel` resolves the first
 * usable model that was not there when it began, so the refused turn can be
 * sent again on it.
 */
import { apiGet } from './api';
import { billingBody } from './billing.mts';
import { appConfig, sessionApi } from './endpoints';
import { useChatStore } from '../stores/chatStore';
import { useSessionStore } from '../stores/sessionStore';
import type { ChatMessage } from '../types';

/** The Linggen Cloud model (provider/models/catalog.rs LINGGEN_CLOUD_MODEL_ID). */
export const CLOUD_MODEL_ID = 'deepseek-flash';

/** A host that can open its settings on a tab: the launcher and the dev
 *  console set it on their window (same origin, so a framed chat reaches it). */
type SettingsHost = (tab: 'models') => void;
declare global {
  interface Window { __linggenOpenSettings?: SettingsHost }
}

function host(): SettingsHost | undefined {
  // Nearest first: a chat inside a skill page inside the launcher asks the launcher.
  const frames: Window[] = [];
  try {
    let w: Window = window;
    frames.push(w);
    while (w.parent && w.parent !== w) { w = w.parent; frames.push(w); }
  } catch { /* a cross-origin ancestor ends the walk */ }
  for (const w of frames) {
    try {
      if (typeof w.__linggenOpenSettings === 'function') return w.__linggenOpenSettings;
    } catch { /* not ours */ }
  }
  return undefined;
}

export function openModelsSettings(): void {
  const open = host();
  if (open) { open('models'); return; }
  window.open('/settings?tab=models', '_blank', 'noopener');
}

interface ModelRow { id: string; auth_ok?: boolean }

/** The models that can answer now, the cloud one aside. */
export async function ownModels(): Promise<string[]> {
  const rows = await apiGet<ModelRow[]>('/api/models').catch(() => [] as ModelRow[]);
  return (Array.isArray(rows) ? rows : []).filter((m) => m.auth_ok && m.id !== CLOUD_MODEL_ID).map((m) => m.id);
}

/** The first own model added after `before` was read, or null once `signal`
 *  aborts or the wait (15 min, as long as a sign-in takes) runs out. */
export async function waitForOwnModel(before: string[], signal: AbortSignal): Promise<string | null> {
  const had = new Set(before);
  const until = Date.now() + 15 * 60_000;
  while (!signal.aborted && Date.now() < until) {
    await new Promise((r) => setTimeout(r, 2500));
    if (signal.aborted) return null;
    const added = (await ownModels()).find((id) => !had.has(id));
    if (added) return added;
  }
  return null;
}

/** Put the added model to work: first in the default chain (Ling's model and
 *  the fallback chain both read it), and this member's model in the session,
 *  so the re-sent turn lands on it instead of the spent cloud. Agent's pick
 *  (Hanli delegated 11-03): choosing "use your own model" makes it the default. */
export async function adoptOwnModel(modelId: string, sessionId?: string | null, agentId?: string): Promise<void> {
  await appConfig.update((c) => {
    const chain = (c.routing?.default_models ?? []).filter((m) => m !== modelId);
    return { ...c, routing: { ...c.routing, default_models: [modelId, ...chain] } };
  }).catch(() => {});
  if (sessionId) await sessionApi.setModel('', sessionId, modelId, agentId).catch(() => {});
}

/** The chat's newest row, when it is a refusal (`BILLING_REQUIRED:`) — an
 *  older one is a turn since answered, never one to send again. */
export function lastRefusal(): ChatMessage | null {
  const rows = useChatStore.getState().messages;
  const last = rows[rows.length - 1];
  return last && last.role !== 'user' && billingBody(last.text) ? last : null;
}

/** The first own model that is usable but not yet first in the default chain:
 *  a model already there, which the spent cloud stood in front of. */
async function standingOwnModel(owned: string[]): Promise<string | null> {
  if (!owned.length) return null;
  const chain = (await appConfig.get().catch(() => null))?.routing?.default_models ?? [];
  return chain[0] && owned.includes(chain[0]) ? null : owned[0];
}

/** The person's message the refusal `row` answered: the last user row before
 *  it (a page's `[HIDDEN]` kickoff stays hidden when sent again). */
function refusedText(row: ChatMessage): string | null {
  const rows = useChatStore.getState().messages;
  const at = rows.indexOf(row);
  for (let i = (at < 0 ? rows.length : at) - 1; i >= 0; i--) {
    if (rows[i].role === 'user') return rows[i].text;
  }
  return null;
}

/** One wait at a time per chat: the chat's card and its page's card share it. */
let waiting: Promise<boolean> | null = null;

/** Wait for an own model to be added, put it to work, then hand `resend` the
 *  refusal row carrying the refused message. Resolves whether it re-sent. */
export function resendOnOwnModel(row: ChatMessage, resend: (failed: ChatMessage) => void, signal: AbortSignal): Promise<boolean> {
  if (waiting) return waiting;
  waiting = (async () => {
    const text = refusedText(row);
    if (!text) return false;
    // A usable own model already there goes to work now; else wait for one added.
    const owned = await ownModels();
    const added = (await standingOwnModel(owned)) ?? await waitForOwnModel(owned, signal);
    if (!added) return false;
    const agentId = row.from || 'ling';
    await adoptOwnModel(added, useSessionStore.getState().activeSessionId, agentId);
    resend({ ...row, resend: { text, agentId, persisted: true } });
    return true;
  })().finally(() => { waiting = null; });
  return waiting;
}

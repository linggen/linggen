/**
 * The ChatGPT sign-in as a provider the person adds (添加服务商 → ChatGPT 订阅).
 * Its row is a key row without the key (`Models`: added models with ✕, one input
 * suggesting ChatGPT's own list), the usage windows, and ⋯ 移除这个服务商 = sign out. It
 * exists only while signed in.
 */
import React, { useState } from 'react';
import { confirmDialog } from '../../lib/confirmDialog';
import { modelsApi, providerAuth, providersApi } from '../../lib/endpoints';
import { W, say } from '../../lib/modelsWords.mts';
import { heldDefaults, type Provider } from '../../lib/providers.mts';
import type { ModelMove } from '../../types';
import { CodexUsage } from '../CodexUsage';
import { Models } from './KeyRow';
import { Row, RowMenu } from './shell';
import type { Run } from './useProviders';

const CODEX = { login: '/api/auth/codex/login', status: '/api/auth/codex/status' };

/** Start the browser sign-in and look every two seconds, for five minutes; true once signed in. */
export async function signInChatgpt(): Promise<boolean> {
  await providerAuth.login(CODEX.login);
  for (let waited = 0; waited < 300_000; waited += 2000) {
    await new Promise((r) => setTimeout(r, 2000));
    const status = await providerAuth.status<{ authenticated: boolean }>(CODEX.status).catch(() => null);
    if (status?.authenticated) return true;
  }
  return false;
}

export const ChatgptRow: React.FC<{
  provider: Provider;
  /** The engine's note on each built-in whose family moved. */
  moved: Record<string, ModelMove | undefined>;
  /** Read the page again after a stay. */
  changed: () => Promise<void>;
  run: Run;
  busy: boolean;
  open: boolean;
  onToggle: () => void;
  /** The default chat chain, first model first: what the remove confirm names. */
  chain: string[];
  makeDefault: (modelId: string) => Promise<void>;
}> = ({ provider: p, moved, changed, run, busy, open, onToggle, chain, makeDefault }) => {
  const removeProvider = async () => {
    const held = heldDefaults(p, chain);
    const lines = [say(W.removeChatgptAsk, { provider: p.label })];
    if (held.first) lines.push(W.removeHoldsDefault);
    if (held.fallbacks.length) lines.push(say(W.removeHoldsFallbacks, { n: held.fallbacks.length }));
    if (await confirmDialog(lines.join(' '))) await run(() => providersApi.remove(p.id));
  };
  const moves = p.models.filter((m) => moved[m.id]);
  const names = p.models.map((m) => m.name).join(' · ');
  const subtitle = (
    <>
      <span>{W.signedIn}</span>
      <span aria-hidden> · </span>
      <span>{W.chatgptNote}</span>
      {names && <><span aria-hidden> · </span><span data-no-i18n>{names}</span></>}
    </>
  );
  return (
    <div>
      <Row testId="chatgpt" name={p.label} way={p.way} subtitle={subtitle} open={open} onToggle={onToggle}
        right={<><CodexUsage /><RowMenu disabled={busy} items={[{ label: W.removeProvider, run: () => void removeProvider() }]} /></>}>
        <Models provider={p} open={open} onToggle={onToggle} run={run} busy={busy} makeDefault={makeDefault} chain={chain} />
      </Row>
      {moves.map((m) => <FamilyMoved key={m.id} move={moved[m.id]!} onChanged={changed} />)}
    </div>
  );
};

/** One line on a ChatGPT built-in whose family moved — to what, since when —
 *  and the one control to stay on the previous model or follow again. */
function FamilyMoved({ move, onChanged }: { move: ModelMove; onChanged: () => Promise<void> }) {
  const [busy, setBusy] = useState(false);
  const date = move.since ? new Date(move.since * 1000).toLocaleDateString() : '';
  const toggle = async () => {
    setBusy(true);
    try {
      await modelsApi.stayChatgpt(move.family, !move.staying);
      await onChanged();
    } finally {
      setBusy(false);
    }
  };
  const line = move.staying
    ? `Staying on ${move.previous}; ${move.current} is newer.`
    : `Moved up from ${move.previous} on ${date}.`;
  const action = move.staying ? `Use ${move.current}` : `Stay on ${move.previous}`;
  return (
    <p className="mx-3 mt-1 text-[11px] text-slate-500">
      <span>{line}</span>
      {(move.staying || move.can_stay) && (
        <button type="button" onClick={() => void toggle()} disabled={busy} className="ml-2 text-[11px] font-semibold text-blue-600 hover:text-blue-700 disabled:opacity-50">
          {action}
        </button>
      )}
    </p>
  );
}

/**
 * Settings → Models: 「Default model」, the Linggen Cloud and ChatGPT rows, then
 * the fold 「Your own providers」. The page draws what
 * the engine answers (`GET /api/providers`) and decides nothing: every button
 * calls its route and the list is redrawn from the answer (models/useProviders).
 * What a key provider shows, and how, is models/KeyRow; the words are
 * lib/modelsWords.mts; reading the answer is lib/providers.mts.
 */
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { ChevronRight, Plus } from 'lucide-react';
import type { ModelMove } from '../types';
import type { UseEdit, Uses } from '../lib/modelUses';
import { useServerStore } from '../stores/serverStore';
import { modelsApi, providersApi, usesApi } from '../lib/endpoints';
import { W, say } from '../lib/modelsWords.mts';
import { hasProblem, keyProviders, localProviders, readyToUse, sameAddress, type Preset, type ProvidersAnswer } from '../lib/providers.mts';
import { AddProvider, type Typed } from './models/AddProvider';
import { DefaultCard } from './models/DefaultCard';
import { KeyRow } from './models/KeyRow';
import { LocalRow } from './models/LocalRow';
import { ReadyRow } from './models/ReadyRows';
import { cardCls, headingCls, hintCls, linkBtn } from './models/shell';
import { useProviders } from './models/useProviders';

export const ModelsTab: React.FC<{
  /** Read the config again: a route of the engine changed it (Settings holds one copy). */
  reload: () => Promise<void>;
}> = ({ reload }) => {
  const { answer, loadFailed, problem, busy, load, run } = useProviders(reload);
  const [uses, setUses] = useState<Uses | null>(null);
  const [usesFailed, setUsesFailed] = useState(false);
  const loadUses = useCallback(async () => {
    try {
      setUses(await usesApi.list());
      setUsesFailed(false);
    } catch {
      setUsesFailed(true);
    }
  }, []);
  useEffect(() => { if (!busy) void loadUses(); }, [answer, busy, loadUses]);
  const [open, setOpen] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  // The own-providers fold: shut unless a provider has a problem, until the person decides.
  const [ownFold, setOwnFold] = useState<boolean | null>(null);
  // A sign-in that made another model the default: one line says so.
  const [signInLine, setSignInLine] = useState<string | null>(null);

  // The engine's note on a ChatGPT built-in whose family moved (`/api/models`).
  const runtimeModels = useServerStore((s) => s.runtimeModels);
  const refreshRuntimeModels = () => { void useServerStore.getState().refreshRuntimeModels(); };
  useEffect(refreshRuntimeModels, []);
  const moved = useMemo(() => Object.fromEntries(runtimeModels.map((m) => [m.id, m.moved as ModelMove | undefined])), [runtimeModels]);

  /** A sign-in, sign-out or stay happened: read the list again, and say what it made the default. */
  const signedInChanged = async () => {
    await run(() => providersApi.list());
    refreshRuntimeModels();
    const made = await modelsApi.signedInDefault().catch(() => null);
    if (!made) return;
    await reload();
    setSignInLine(made.line);
  };

  const pickUse = (edit: UseEdit) => run(async () => {
    setUses(await usesApi.put(edit));
    setUsesFailed(false);
    if (edit.use === 'chat' || edit.use === 'chat_fallbacks') return providersApi.list();
  }).then(() => undefined);
  const makeDefault = (choice: string) => pickUse({ use: 'chat', choice });

  /** A preset and its key: the provider is filed only now, with the key; the row opens. */
  const addPreset = async (preset: Preset, key: string) => {
    const find = (a: ProvidersAnswer) => a.providers.find((p) => p.way === 'key' && p.provider === preset.name && sameAddress(p.url, preset.url));
    let made: ProvidersAnswer | void = undefined;
    const done = await run(async () => {
      made = await providersApi.create(preset.label);
      const row = find(made);
      return row ? (made = await providersApi.setKey(row.id, key)) : made;
    });
    const row = done && made ? find(made as ProvidersAnswer) : undefined;
    if (row) setOpen(row.id);
    if (done) setAdding(false);
  };

  /** 其他…: file the typed provider, then its key; the row opens. */
  const addTyped = async (typed: Typed) => {
    let made: ProvidersAnswer | void = undefined;
    const done = await run(async () => {
      made = await providersApi.createTyped(typed.name, typed.url, typed.style);
      const row = made.providers.find((p) => p.way === 'key' && sameAddress(p.url, typed.url));
      return row ? providersApi.setKey(row.id, typed.key) : made;
    });
    const row = done && made ? (made as ProvidersAnswer).providers.find((p) => p.way === 'key' && sameAddress(p.url, typed.url)) : undefined;
    if (row) setOpen(row.id);
    if (done) setAdding(false);
  };

  if (loadFailed && !answer) {
    return (
      <section className={cardCls}>
        <p data-testid="providers-load-failed" className="text-xs text-red-600 dark:text-red-400">
          <span>{W.providersFailed}</span>{' '}
          <button type="button" className="underline" onClick={() => void load()}>{W.retry}</button>
        </p>
      </section>
    );
  }
  if (!answer) return <p className={hintCls}>{W.loading}</p>;

  const { providers } = answer;
  const chain = uses?.chain ?? [];
  const addable = answer.presets.filter((p) => p.way === 'key' && !providers.some((q) => q.way === 'key' && q.provider === p.name && sameAddress(q.url, p.url)));
  const local = localProviders(providers);
  const ownOpen = ownFold ?? providers.some(hasProblem);
  const connected = [...keyProviders(providers), ...local].filter((p) => p.connected).length;
  return (
    <div className="space-y-6" data-testid="models-page">
      <DefaultCard view={uses} busy={busy} failed={usesFailed} retry={() => void loadUses()} pick={(edit) => void pickUse(edit)} />
      {problem && <p role="alert" data-testid="models-problem" className="text-xs text-red-600 dark:text-red-400">{problem}</p>}
      {signInLine && <p data-testid="signed-in-default" className="text-xs text-green-700 dark:text-green-400">{signInLine}</p>}
      <section className={cardCls} data-testid="ready-to-use">
        <div className="space-y-1.5">
          {readyToUse(providers).map((p) => <ReadyRow key={p.id} provider={p} moved={moved} changed={signedInChanged} run={run} busy={busy} makeDefault={makeDefault} open={open === p.id} onToggle={() => setOpen(open === p.id ? null : p.id)} />)}
        </div>
      </section>
      <section className={cardCls} data-testid="own-providers">
        <button type="button" data-testid="own-providers-toggle" aria-expanded={ownOpen} onClick={() => setOwnFold(!ownOpen)} className="flex w-full items-center gap-2 text-left">
          <ChevronRight aria-hidden size={12} className={`shrink-0 text-slate-400 transition-transform ${ownOpen ? 'rotate-90' : ''}`} />
          <h2 className={headingCls}>{W.ownProviders}</h2>
          <span className={hintCls}>{say(W.nConnected, { n: connected })}</span>
        </button>
        {ownOpen && (
          <div className="mt-3">
            <div className="flex flex-wrap items-baseline justify-between gap-2">
              <p className={`${hintCls} min-w-0`}>{W.providersHint}</p>
              <button type="button" data-testid="add-provider-button" className={`${linkBtn} flex items-center gap-1`} onClick={() => setAdding(!adding)}>
                <Plus size={12} />{W.addProvider}
              </button>
            </div>
            <div className="mt-4">
              {adding && <AddProvider presets={addable} busy={busy} onAdd={(p, k) => void addPreset(p, k)} onTyped={(t) => void addTyped(t)} onClose={() => setAdding(false)} />}
              <div className="space-y-1.5">
                {keyProviders(providers).map((p) => (
                  <KeyRow key={p.id} provider={p} open={open === p.id} onToggle={() => setOpen(open === p.id ? null : p.id)} run={run} busy={busy} makeDefault={makeDefault} chain={chain} />
                ))}
                {local.length > 0 && <LocalRow providers={local} />}
              </div>
            </div>
          </div>
        )}
      </section>
    </div>
  );
};

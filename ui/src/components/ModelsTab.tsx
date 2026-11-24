/**
 * Settings → Models: 「Default model」 above 「Providers」. The page draws what
 * the engine answers (`GET /api/providers`) and decides nothing: every button
 * calls its route and the list is redrawn from the answer (models/useProviders).
 * What a key provider shows, and how, is models/KeyRow; the words are
 * lib/modelsWords.mts; reading the answer is lib/providers.mts.
 */
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { Plus } from 'lucide-react';
import type { ModelMove } from '../types';
import type { UseEdit, Uses } from '../lib/modelUses';
import { useServerStore } from '../stores/serverStore';
import { modelsApi, providersApi, usesApi } from '../lib/endpoints';
import { W } from '../lib/modelsWords.mts';
import { keyProviders, localProviders, readyToUse, sameAddress, type Preset, type ProvidersAnswer } from '../lib/providers.mts';
import { AddProvider } from './models/AddProvider';
import { DefaultCard } from './models/DefaultCard';
import { KeyRow } from './models/KeyRow';
import { LocalRow } from './models/LocalRow';
import { ReadyRow } from './models/ReadyRows';
import { cardCls, hintCls, linkBtn } from './models/shell';
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
    if (edit.use === 'chat' || edit.use === 'live') return providersApi.list();
  }).then(() => undefined);
  const makeDefault = (choice: string) => pickUse({ use: 'chat', choice });

  const pick = async (preset: Preset) => {
    let made: ProvidersAnswer | void = undefined;
    const done = await run(async () => (made = await providersApi.create(preset.label)));
    const row = done && made ? (made as ProvidersAnswer).providers.find((p) => p.way === 'key' && p.provider === preset.name && sameAddress(p.url, preset.url)) : undefined;
    if (row) setOpen(row.id);
    setAdding(false);
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
  const addable = answer.presets.filter((p) => p.way === 'key' && !providers.some((q) => q.way === 'key' && q.provider === p.name && sameAddress(q.url, p.url)));
  const local = localProviders(providers);
  return (
    <div className="space-y-6" data-testid="models-page">
      <DefaultCard view={uses} busy={busy} failed={usesFailed} retry={() => void loadUses()} pick={(edit) => void pickUse(edit)} />
      <section className={cardCls}>
        <div className="flex flex-wrap items-start justify-between gap-2">
          <div className="min-w-0">
            <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300">{W.providers}</h2>
            <p className={`${hintCls} mt-1`}>{W.providersHint}</p>
          </div>
          <button type="button" data-testid="add-provider-button" className={`${linkBtn} flex items-center gap-1`} onClick={() => setAdding(!adding)}>
            <Plus size={12} />{W.addProvider}
          </button>
        </div>
        {problem && <p role="alert" data-testid="models-problem" className="mt-3 text-xs text-red-600 dark:text-red-400">{problem}</p>}
        {signInLine && <p data-testid="signed-in-default" className="mt-3 text-xs text-green-700 dark:text-green-400">{signInLine}</p>}
        <div className="mt-4">
          {adding && <AddProvider presets={addable} busy={busy} onPick={(p) => void pick(p)} onClose={() => setAdding(false)} />}
          <GroupHead title={W.readyToUse} hint={W.readyToUseHint} />
          <div className="space-y-1.5" data-testid="ready-to-use">
            {readyToUse(providers).map((p) => <ReadyRow key={p.id} provider={p} moved={moved} changed={signedInChanged} />)}
          </div>
          <GroupHead title={W.ownKeys} hint={W.ownKeysHint} />
          <div className="space-y-1.5">
            {keyProviders(providers).map((p) => (
              <KeyRow key={p.id} provider={p} open={open === p.id} onToggle={() => setOpen(open === p.id ? null : p.id)} run={run} busy={busy} makeDefault={makeDefault} />
            ))}
            {local.length > 0 && <LocalRow providers={local} />}
          </div>
        </div>
      </section>
    </div>
  );
};

const GroupHead: React.FC<{ title: string; hint: string }> = ({ title, hint }) => (
  <div className="mb-2 mt-4 flex flex-wrap items-baseline gap-2 first:mt-0">
    <h3 className="text-[11px] font-bold text-slate-500 dark:text-slate-400">{title}</h3>
    <span className="text-[11px] text-slate-400">{hint}</span>
  </div>
);

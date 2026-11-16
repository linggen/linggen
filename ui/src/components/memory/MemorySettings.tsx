import React, { useCallback, useEffect, useState } from 'react';
import { LingMemPrimary } from '../settings/LingMemPrimary';
import { describeError, MemoryError } from './memoryApi.mts';
import {
  draftOf, isChanged, parseDraft, readSettings, saveSettings, TTL_MAX, TTL_MIN,
  type Draft, type MemorySettings as Settings,
} from './memoryConfigApi.mts';
import { ghostBtn, inputCls, labelCls, primaryBtn, sectionCls } from './memoryStyle';

const OWNER_ONLY = "Only this Mac's owner can change these.";
const isForbidden = (e: unknown) => e instanceof MemoryError && e.forbidden;

/** Memory → Settings: how long short-term rows are kept, how selective recall
 *  is, and where memory lives when more than one machine shares it. */
export const MemorySettings: React.FC = () => {
  const [stored, setStored] = useState<Settings | null>(null);
  const [draft, setDraft] = useState<Draft>({ days: '', score: '' });
  const [error, setError] = useState<unknown>(null);
  const [problem, setProblem] = useState('');
  const [saved, setSaved] = useState(false);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try { const s = await readSettings(); setStored(s); setDraft(draftOf(s)); setError(null); }
    catch (e) { setError(e); }
  }, []);
  useEffect(() => { void load(); }, [load]);

  const edit = (patch: Partial<Draft>) => { setDraft((d) => ({ ...d, ...patch })); setProblem(''); setSaved(false); };
  const save = async () => {
    const parsed = parseDraft(draft);
    if ('error' in parsed) { setProblem(parsed.error); return; }
    setBusy(true); setSaved(false);
    try { const s = await saveSettings(parsed.settings); setStored(s); setDraft(draftOf(s)); setError(null); setSaved(true); }
    catch (e) { setError(e); }
    finally { setBusy(false); }
  };

  const forbidden = isForbidden(error);
  return (
    <div className="space-y-4" data-testid="memory-subview-settings">
      {forbidden && <p role="status" className="text-xs text-slate-500 dark:text-slate-400">{OWNER_ONLY}</p>}
      {error != null && !forbidden && (
        <div role="alert" className="text-xs text-red-600 dark:text-red-400">
          {describeError(error, stored ? 'save the settings' : 'read the settings')}
          {!stored && <button className={ghostBtn} onClick={load}>Try again</button>}
        </div>
      )}
      {!stored && error == null && <p className="text-xs text-slate-500">Loading…</p>}
      {stored && (
        <section className={`${sectionCls} space-y-4`}>
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
            <div>
              <label className={labelCls} htmlFor="memory-ttl-days">Keep short-term rows for (days)</label>
              <input id="memory-ttl-days" className={`${inputCls} mt-1`} type="number" inputMode="numeric" min={TTL_MIN} max={TTL_MAX} step={1}
                value={draft.days} onChange={(e) => edit({ days: e.target.value })} />
              <p className="text-[11px] text-slate-400 mt-1">Short-term rows older than this are forgotten once a dream has judged them.</p>
            </div>
            <div>
              <label className={labelCls} htmlFor="memory-min-score">Recall threshold (0–1)</label>
              <input id="memory-min-score" className={`${inputCls} mt-1`} type="number" inputMode="decimal" min={0} max={1} step={0.05}
                value={draft.score} onChange={(e) => edit({ score: e.target.value })} />
              <p className="text-[11px] text-slate-400 mt-1">Higher brings back fewer, closer matches. Lower brings back more.</p>
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-3">
            <button className={primaryBtn} disabled={busy || !isChanged(draft, stored)} onClick={save}>{busy ? 'Saving…' : 'Save'}</button>
            {saved && <span role="status" className="text-xs text-slate-500">Saved</span>}
            {problem && <span role="alert" className="text-xs text-red-600 dark:text-red-400">{problem}</span>}
          </div>
        </section>
      )}
      {!forbidden && (
        <section className={`${sectionCls} space-y-2`}>
          <h3 className="text-xs font-bold text-slate-700 dark:text-slate-300">Other machines</h3>
          <p className="text-[11px] text-slate-400">Where memory is kept: this machine, or another on your LAN.</p>
          <LingMemPrimary />
          <p className="text-[11px] text-slate-400">A replica keeps a copy here, syncs, and works offline. Type local to hold it here.</p>
        </section>
      )}
    </div>
  );
};

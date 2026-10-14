/**
 * Settings → Weather: the live sky background (this browser), the place the
 * weather is read at (found by IP, or typed), and the weather sense itself.
 */
import React, { useEffect, useState } from 'react';
import { placeLine } from '../lib/skyChoice.mts';
import { useSkyStore } from '../stores/skyStore';

const inputCls = 'flex-1 min-w-0 bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded-lg px-3 py-2 text-xs outline-none focus:ring-1 focus:ring-blue-500/50';
const btnCls = 'px-3 py-2 rounded-lg text-xs font-semibold border border-slate-200 dark:border-white/10 hover:bg-slate-100 dark:hover:bg-white/5 disabled:opacity-50';
const labelCls = 'text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400';

const Toggle: React.FC<{ label: string; hint: string; on: boolean; onChange: (on: boolean) => void }> = ({ label, hint, on, onChange }) => (
  <label className="flex items-start justify-between gap-4 cursor-pointer">
    <span>
      <span className="block text-xs font-semibold text-slate-700 dark:text-slate-200">{label}</span>
      <span className="block text-[11px] text-slate-500 dark:text-slate-400 mt-0.5">{hint}</span>
    </span>
    <input type="checkbox" className="mt-0.5" checked={on} onChange={(e) => onChange(e.target.checked)} />
  </label>
);

export const WeatherSettings: React.FC<{ className?: string }> = ({ className }) => {
  const { enabled, view, error, setEnabled, load, setCity, setOff } = useSkyStore();
  const [typed, setTyped] = useState('');
  const [busy, setBusy] = useState(false);
  useEffect(() => { load(); }, [load]);

  const run = async (act: () => Promise<void>) => {
    setBusy(true);
    await act();
    setBusy(false);
  };
  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    const name = typed.trim();
    if (name) run(() => setCity(name)).then(() => setTyped(''));
  };
  const off = !!view?.off;

  return (
    <section className={className}>
      <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300 mb-4">Weather</h2>
      <div className="space-y-4">
        <Toggle label="Live sky background" hint="The weather outside, behind Linggen." on={enabled} onChange={setEnabled} />
        <Toggle label="Read the weather" hint="Off: nothing is fetched, not even your city." on={!off} onChange={(on) => run(() => setOff(!on))} />
        <div>
          <div className={labelCls}>Place</div>
          <div className="mt-1 text-xs text-slate-700 dark:text-slate-200">{placeLine(view)}</div>
          {!off && (
            <form onSubmit={submit} className="mt-2 flex gap-2">
              <input className={inputCls} value={typed} onChange={(e) => setTyped(e.target.value)} placeholder="Type a city" maxLength={80} />
              <button type="submit" className={btnCls} disabled={busy || !typed.trim()}>Set</button>
              {view && !view.auto && (
                <button type="button" className={btnCls} disabled={busy} onClick={() => run(() => setCity(null))}>Automatic</button>
              )}
            </form>
          )}
          {error && <p className="text-[11px] text-red-500 mt-1">{error}</p>}
        </div>
      </div>
    </section>
  );
};

/**
 * 「Add provider」: pick one of the engine's presets (or 其他… to type a name,
 * an address and an API style), paste the key, then 添加. Nothing is added to
 * the list until 添加 with a key: a pick only selects. The page has no list of
 * its own.
 */
import React, { useState } from 'react';
import { W } from '../../lib/modelsWords.mts';
import type { ApiStyle, Preset } from '../../lib/providers.mts';
import { inputCls, linkBtn, primaryBtn } from './shell';

export type Typed = { name: string; url: string; style: ApiStyle; key: string };

const chip = 'flex min-w-0 items-center justify-between gap-1.5 rounded-lg border px-2.5 py-1.5 text-left text-xs disabled:opacity-50';
const chipOff = 'border-slate-200 bg-white text-slate-700 hover:border-blue-300 dark:border-white/10 dark:bg-white/5 dark:text-slate-200 dark:hover:border-blue-700';
const chipOn = 'border-blue-500 bg-blue-50 text-blue-700 dark:border-blue-500 dark:bg-blue-500/10 dark:text-blue-300';

export const AddProvider: React.FC<{ presets: Preset[]; busy: boolean; onAdd: (preset: Preset, key: string) => void; onTyped: (typed: Typed) => void; onClose: () => void }> = ({ presets, busy, onAdd, onTyped, onClose }) => {
  // The chip picked: a preset's index, 'other', or nothing yet.
  const [picked, setPicked] = useState<number | 'other' | null>(null);
  const [typed, setTyped] = useState<Typed>({ name: '', url: '', style: 'openai', key: '' });
  const set = (field: keyof Typed) => (e: React.ChangeEvent<HTMLInputElement | HTMLSelectElement>) => setTyped({ ...typed, [field]: e.target.value });
  const key = typed.key.trim();
  const preset = typeof picked === 'number' ? presets[picked] : undefined;
  const ready = picked === 'other' ? !!(typed.name.trim() && typed.url.trim() && key) : !!(preset && key);
  const add = () => {
    if (!ready) return;
    if (preset) onAdd(preset, key);
    else onTyped({ ...typed, name: typed.name.trim(), url: typed.url.trim(), key });
  };
  return (
    <div data-testid="add-provider" className="mb-4 rounded-xl border border-slate-200 bg-slate-50 p-3 dark:border-white/10 dark:bg-white/[0.02]">
      <div className="flex items-center gap-2">
        <span className="text-xs font-bold">{W.pickProvider}</span>
        <button type="button" className={`${linkBtn} ml-auto`} onClick={onClose}>{W.close}</button>
      </div>
      <div className="mt-2 grid grid-cols-[repeat(auto-fill,minmax(9rem,1fr))] gap-1.5">
        {presets.map((p, i) => (
          <button key={`${p.name}${p.url}`} type="button" aria-pressed={picked === i} disabled={busy} onClick={() => setPicked(i)} className={`${chip} ${picked === i ? chipOn : chipOff}`} data-no-i18n>
            <span className="truncate">{p.label}</span>
          </button>
        ))}
        <button type="button" data-testid="add-other" aria-pressed={picked === 'other'} disabled={busy} onClick={() => setPicked('other')} className={`${chip} border-dashed ${picked === 'other' ? chipOn : chipOff}`}>
          <span className="truncate">{W.otherProvider}</span>
        </button>
      </div>
      {picked !== null && (
        <div data-testid="add-provider-form" className="mt-3 grid gap-2 sm:grid-cols-2">
          {picked === 'other' && (
            <>
              <input className={inputCls} placeholder={W.providerName} aria-label={W.providerName} value={typed.name} onChange={set('name')} />
              <input className={`${inputCls} font-mono`} placeholder={W.providerAddress} aria-label={W.providerAddress} value={typed.url} onChange={set('url')} />
              <select className={`${inputCls} dark:[color-scheme:dark]`} aria-label={W.apiStyle} value={typed.style} onChange={set('style')}>
                <option value="openai">{W.styleOpenai}</option>
                <option value="claude">{W.styleClaude}</option>
              </select>
            </>
          )}
          <input type="password" autoComplete="off" className={inputCls} placeholder={W.pasteKey} aria-label={W.pasteKey} value={typed.key} onChange={set('key')} onKeyDown={(e) => { if (e.key === 'Enter') add(); }} />
          <div className="sm:col-span-2">
            <button type="button" data-testid="add-provider-confirm" className={primaryBtn} disabled={busy || !ready} onClick={add}>{W.add}</button>
          </div>
        </div>
      )}
    </div>
  );
};

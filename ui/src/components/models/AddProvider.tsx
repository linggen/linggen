/**
 * 「Add provider」: the engine's preset table, one pick, and the row opens with
 * its one key field. 其他… is a provider the person types: a name, an address,
 * an API style and the key. The page has no list of its own.
 */
import React, { useState } from 'react';
import { ChevronRight } from 'lucide-react';
import { W } from '../../lib/modelsWords.mts';
import type { ApiStyle, Preset } from '../../lib/providers.mts';
import { inputCls, linkBtn, primaryBtn } from './shell';

export type Typed = { name: string; url: string; style: ApiStyle; key: string };

export const AddProvider: React.FC<{ presets: Preset[]; busy: boolean; onPick: (preset: Preset) => void; onTyped: (typed: Typed) => void; onClose: () => void }> = ({ presets, busy, onPick, onTyped, onClose }) => {
  const [other, setOther] = useState(false);
  const [typed, setTyped] = useState<Typed>({ name: '', url: '', style: 'openai', key: '' });
  const set = (field: keyof Typed) => (e: React.ChangeEvent<HTMLInputElement | HTMLSelectElement>) => setTyped({ ...typed, [field]: e.target.value });
  const ready = typed.name.trim() && typed.url.trim() && typed.key.trim();
  return (
    <div data-testid="add-provider" className="mb-4 rounded-xl border border-slate-200 bg-slate-50 p-3 dark:border-white/10 dark:bg-white/[0.02]">
      <div className="flex items-center gap-2">
        <span className="text-xs font-bold">{W.pickProvider}</span>
        <button type="button" className={`${linkBtn} ml-auto`} onClick={onClose}>{W.close}</button>
      </div>
      <div className="mt-2 grid grid-cols-[repeat(auto-fill,minmax(9rem,1fr))] gap-1.5">
        {presets.map((p) => (
          <button key={`${p.name}${p.url}`} type="button" disabled={busy} onClick={() => onPick(p)} className="flex min-w-0 items-center justify-between gap-1.5 rounded-lg border border-slate-200 bg-white px-2.5 py-1.5 text-left text-xs text-slate-700 hover:border-blue-300 disabled:opacity-50 dark:border-white/10 dark:bg-white/5 dark:text-slate-200 dark:hover:border-blue-700" data-no-i18n>
            <span className="truncate">{p.label}</span><ChevronRight aria-hidden size={12} className="shrink-0 text-slate-400" />
          </button>
        ))}
        <button type="button" data-testid="add-other" disabled={busy} onClick={() => setOther(!other)} className="flex min-w-0 items-center justify-between gap-1.5 rounded-lg border border-dashed border-slate-300 bg-white px-2.5 py-1.5 text-left text-xs text-slate-700 hover:border-blue-300 disabled:opacity-50 dark:border-white/15 dark:bg-white/5 dark:text-slate-200">
          <span className="truncate">{W.otherProvider}</span><ChevronRight aria-hidden size={12} className={`shrink-0 text-slate-400 ${other ? 'rotate-90' : ''}`} />
        </button>
      </div>
      {other && (
        <div data-testid="other-provider" className="mt-3 grid gap-2 sm:grid-cols-2">
          <input className={inputCls} placeholder={W.providerName} aria-label={W.providerName} value={typed.name} onChange={set('name')} />
          <input className={`${inputCls} font-mono`} placeholder={W.providerAddress} aria-label={W.providerAddress} value={typed.url} onChange={set('url')} />
          <select className={`${inputCls} dark:[color-scheme:dark]`} aria-label={W.apiStyle} value={typed.style} onChange={set('style')}>
            <option value="openai">{W.styleOpenai}</option>
            <option value="claude">{W.styleClaude}</option>
          </select>
          <input type="password" autoComplete="off" className={inputCls} placeholder={W.pasteKey} aria-label={W.pasteKey} value={typed.key} onChange={set('key')} />
          <div className="sm:col-span-2">
            <button type="button" className={primaryBtn} disabled={busy || !ready} onClick={() => onTyped({ ...typed, name: typed.name.trim(), url: typed.url.trim(), key: typed.key.trim() })}>{W.add}</button>
          </div>
        </div>
      )}
    </div>
  );
};

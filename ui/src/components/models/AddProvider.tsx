/**
 * 「Add provider」: the engine's preset list, one pick, and the row opens with
 * its one key field. The list is `presets` as the engine answered it; the page
 * has no list of its own.
 */
import React from 'react';
import { W } from '../../lib/modelsWords.mts';
import type { Preset } from '../../lib/providers.mts';
import { linkBtn } from './shell';

export const AddProvider: React.FC<{ presets: Preset[]; busy: boolean; onPick: (preset: Preset) => void; onClose: () => void }> = ({ presets, busy, onPick, onClose }) => (
  <div data-testid="add-provider" className="mb-4 rounded-xl border border-slate-200 bg-slate-50 p-3 dark:border-white/10 dark:bg-white/[0.02]">
    <div className="flex items-center gap-2">
      <span className="text-xs font-bold">{W.pickProvider}</span>
      <button type="button" className={`${linkBtn} ml-auto`} onClick={onClose}>{W.close}</button>
    </div>
    <div className="mt-2 grid grid-cols-[repeat(auto-fill,minmax(9rem,1fr))] gap-1.5">
      {presets.map((p) => (
        <button key={`${p.name}${p.url}`} type="button" disabled={busy} onClick={() => onPick(p)} className="min-w-0 truncate rounded-lg border border-slate-200 bg-white px-2.5 py-2 text-left text-xs hover:border-blue-300 disabled:opacity-50 dark:border-white/10 dark:bg-white/5" data-no-i18n>
          {p.label}
        </button>
      ))}
    </div>
  </div>
);

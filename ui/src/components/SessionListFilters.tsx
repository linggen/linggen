import React from 'react';
import { cn } from '../lib/cn';

export type SessionType = 'user' | 'external' | 'mission' | 'skill';
const types: { id: SessionType; label: string }[] = [
  { id: 'user', label: '我的' }, { id: 'external', label: 'External' },
  { id: 'mission', label: 'Mission' }, { id: 'skill', label: 'Skill' },
];
const typesKey = 'linggen:session-history-types';
const archivedKey = 'linggen:session-history-ids';

export function readHistoryTypes(): SessionType[] {
  try {
    const stored = localStorage.getItem(typesKey);
    if (stored !== null) {
      const parsed: unknown = JSON.parse(stored);
      if (Array.isArray(parsed)) return types.map((t) => t.id).filter((id) => parsed.includes(id));
    }
  } catch { /* Storage may be unavailable. */ }
  return ['user', 'external'];
}

export function readArchived(): Set<string> {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(archivedKey) || '[]');
    if (Array.isArray(parsed)) return new Set(parsed.filter((id): id is string => typeof id === 'string'));
  } catch { /* Storage may be unavailable. */ }
  return new Set();
}

export function saveArchived(ids: Set<string>) {
  try { localStorage.setItem(archivedKey, JSON.stringify([...ids])); } catch { /* Keep the current view usable. */ }
}

export const HistoryFilters: React.FC<{
  types: SessionType[];
  onChange: (types: SessionType[]) => void;
  selectMode: boolean;
  onSelectType: (type: SessionType) => void;
}> = ({ types: selected, onChange, selectMode, onSelectType }) => (
  <div className="flex flex-wrap gap-1 px-3 py-2 border-b border-slate-100 dark:border-white/5">
    {types.map(({ id, label }) => <div key={id} className="flex items-center">
      <button aria-pressed={selected.includes(id)} onClick={() => {
        const next = selected.includes(id) ? selected.filter((t) => t !== id) : [...selected, id];
        onChange(next);
        try { localStorage.setItem(typesKey, JSON.stringify(next)); } catch { /* Keep the current view usable. */ }
      }} className={cn('px-2 py-1 rounded-full text-[11px] font-medium transition-colors', selected.includes(id)
        ? 'bg-blue-100 text-blue-700 dark:bg-blue-500/15 dark:text-blue-300'
        : 'bg-slate-100 text-slate-400 dark:bg-white/5 dark:text-slate-500')}>
        {label}
      </button>
      {selectMode && selected.includes(id) && <button onClick={() => onSelectType(id)}
        aria-label={`选择此类型的全部${label}会话`} title={`选择此类型的全部${label}会话`}
        className="px-1 text-xs text-blue-500 hover:text-blue-700">✓</button>}
    </div>)}
  </div>
);

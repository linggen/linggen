import React, { useState } from 'react';
import { MoreHorizontal, Plus, Search, X } from 'lucide-react';
import { ORIGINS, TYPES, type Sort, type TabKey } from './memoryApi.mts';
import type { Query } from './useMemoryRows';
import { ghostBtn, inputCls, primaryBtn } from './memoryStyle';
import { rowWord } from './memoryFormat.mts';

const TABS: { key: TabKey; label: string }[] = [
  { key: 'all', label: 'All' },
  { key: 'core', label: 'Core' },
  { key: 'semantic', label: 'Long-term' },
  { key: 'episodic', label: 'Episodic' },
];

interface Props {
  query: Query;
  checkedCount: number;
  onChange: (patch: Partial<Query>) => void;
  onAdd: () => void;
  onForget: () => void;
  onDeleteChecked: () => void;
  onClearChecked: () => void;
}

/** Tier tabs, the search box, the filters and the actions. */
export const MemoryToolbar: React.FC<Props> = (p) => {
  const { query } = p;
  return (
    <div className="space-y-2.5">
      <div className="flex flex-wrap items-center gap-2">
        <nav className="flex gap-1" role="tablist" aria-label="Memory tier">
          {TABS.map((t) => (
            <button
              key={t.key}
              role="tab"
              aria-selected={query.tab === t.key}
              onClick={() => p.onChange({ tab: t.key })}
              className={`px-2 md:px-3 py-1.5 min-h-11 md:min-h-0 rounded-lg text-xs font-semibold transition-colors ${
                query.tab === t.key
                  ? 'bg-blue-600/10 text-blue-600 dark:text-blue-400'
                  : 'text-slate-500 hover:text-slate-700 dark:hover:text-slate-300 hover:bg-slate-100 dark:hover:bg-white/5'
              }`}
            >
              {t.label}
            </button>
          ))}
        </nav>
        <span className="flex-1" />
        <button className={primaryBtn} onClick={p.onAdd}><Plus size={14} /> Add</button>
        <MoreMenu {...p} />
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <SearchBox text={query.text} onSubmit={(text) => p.onChange({ text })} />
        <Choice label="Type" value={query.type} options={TYPES} onChange={(type) => p.onChange({ type })} />
        <Choice label="From" value={query.from} options={ORIGINS} onChange={(from) => p.onChange({ from })} />
        {!query.text && (
          <select aria-label="Sort" className={`${inputCls} min-h-11 md:min-h-0 !w-auto`} value={query.sort} onChange={(e) => p.onChange({ sort: e.target.value as Sort })}>
            <option value="newest">Newest</option>
            <option value="oldest">Oldest</option>
          </select>
        )}
        {p.checkedCount > 0 && <span className="text-xs text-slate-500">{`${p.checkedCount} selected`}</span>}
      </div>
    </div>
  );
};

const SearchBox: React.FC<{ text: string; onSubmit: (text: string) => void }> = ({ text, onSubmit }) => {
  const [typed, setTyped] = useState(text);
  return (
    <div className="relative flex-1 min-w-48">
      <Search size={13} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-slate-400" />
      <input
       
        className={`${inputCls} min-h-11 md:min-h-0 pl-8 pr-8`}
        placeholder="Search by meaning"
        aria-label="Search memory"
        value={typed}
        onChange={(e) => setTyped(e.target.value)}
        onKeyDown={(e) => {
          if (e.nativeEvent.isComposing || e.keyCode === 229) return;
          if (e.key === 'Enter') onSubmit(typed.trim());
        }}
      />
      {typed && (
        <button aria-label="Clear search" className="absolute right-2 top-1/2 -translate-y-1/2 text-slate-400 hover:text-slate-600" onClick={() => { setTyped(''); onSubmit(''); }}>
          <X size={14} />
        </button>
      )}
    </div>
  );
};

const Choice: React.FC<{ label: string; value: string; options: readonly string[]; onChange: (v: string) => void }> = ({ label, value, options, onChange }) => (
  <select aria-label={label} className={`${inputCls} min-h-11 md:min-h-0 !w-auto`} value={value} onChange={(e) => onChange(e.target.value)}>
    <option value="">{`${label}: any`}</option>
    {options.map((o) => <option key={o} value={o}>{rowWord(o)}</option>)}
  </select>
);

const MoreMenu: React.FC<Props> = (p) => {
  const [open, setOpen] = useState(false);
  const pick = (fn: () => void) => () => { setOpen(false); fn(); };
  return (
    <div className="relative">
      <button className={`${ghostBtn} min-w-11 justify-center md:min-w-0`} aria-label="More actions" aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <MoreHorizontal size={14} />
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-10" onClick={() => setOpen(false)} />
          <div role="menu" className="absolute right-0 z-20 mt-1 w-52 rounded-lg border border-slate-200 dark:border-white/10 bg-white dark:bg-[#1c1c1c] shadow-lg py-1 text-xs">
            <Item disabled={p.checkedCount === 0} danger onClick={pick(p.onDeleteChecked)}>{`Delete selected (${p.checkedCount})`}</Item>
            <Item disabled={p.checkedCount === 0} onClick={pick(p.onClearChecked)}>Clear selection</Item>
            <Item danger onClick={pick(p.onForget)}>Forget by filter…</Item>
          </div>
        </>
      )}
    </div>
  );
};

const Item: React.FC<{ onClick: () => void; disabled?: boolean; danger?: boolean; children: React.ReactNode }> = ({ onClick, disabled, danger, children }) => (
  <button
    role="menuitem"
    disabled={disabled}
    onClick={onClick}
    className={`w-full text-left px-3 py-3.5 md:py-1.5 hover:bg-slate-100 dark:hover:bg-white/5 disabled:opacity-40 disabled:hover:bg-transparent ${danger ? 'text-red-600 dark:text-red-400' : 'text-slate-700 dark:text-slate-200'}`}
  >
    {children}
  </button>
);

/** The look every part of Settings → Models shares: the cards, a provider row's head, a small menu. */
import React, { useEffect, useRef, useState } from 'react';
import { ChevronRight, MoreHorizontal } from 'lucide-react';
import { W } from '../../lib/modelsWords.mts';
import type { Way } from '../../lib/providers.mts';

export const cardCls = 'bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm p-4 md:p-5';
export const inputCls = 'min-w-0 bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded-lg px-3 py-1.5 text-xs text-slate-700 dark:text-slate-200 outline-none focus:ring-1 focus:ring-blue-500/50';
export const primaryBtn = 'px-3 py-1.5 text-xs font-semibold rounded-lg bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50 whitespace-nowrap';
export const ghostBtn = 'px-3 py-1.5 text-xs font-semibold rounded-lg border border-slate-200 dark:border-white/10 text-slate-700 dark:text-slate-200 hover:bg-slate-50 dark:hover:bg-white/5 disabled:opacity-50 whitespace-nowrap';
export const linkBtn = 'text-[11px] font-semibold text-blue-600 hover:text-blue-700 dark:text-blue-400 dark:hover:text-blue-300 disabled:opacity-50';
export const headingCls = 'text-xs font-bold tracking-wide text-slate-700 dark:text-slate-300';
export const sectionLabel = 'w-16 shrink-0 text-[11px] font-semibold text-slate-500 dark:text-slate-400';
export const sectionRule = 'border-t border-slate-100 py-2 dark:border-white/5';
export const hintCls = 'text-[11px] text-slate-500 dark:text-slate-400';

/** A short fixed label beside a name: Account, Key, Pinned… */
export const Tag: React.FC<{ tone?: 'slate' | 'green' | 'blue' | 'amber'; children: React.ReactNode }> = ({ tone = 'slate', children }) => {
  const tones = {
    slate: 'border-slate-300 text-slate-600 dark:border-white/15 dark:text-slate-300',
    green: 'border-emerald-300 text-emerald-700 dark:border-emerald-700/60 dark:text-emerald-400',
    blue: 'border-blue-300 text-blue-600 dark:border-blue-700/60 dark:text-blue-400',
    amber: 'border-amber-300 text-amber-700 dark:border-amber-700/60 dark:text-amber-400',
  } as const;
  return <span className={`shrink-0 whitespace-nowrap rounded border px-1.5 text-[10px] font-bold leading-[15px] ${tones[tone]}`}>{children}</span>;
};

const connection = {
  key: { word: W.kindKey, tone: 'blue' },
  local: { word: W.kindLocal, tone: 'slate' },
  account: { word: W.kindAccount, tone: 'slate' },
  subscription: { word: W.kindSubscription, tone: 'green' },
} as const;

export function ConnectionTag({ way }: { way: Way }) {
  const { word, tone } = connection[way];
  return <Tag tone={tone}>{word}</Tag>;
}

/** One provider: a head, and what is under it. A head that opens is one button. */
export const Row: React.FC<{
  testId: string;
  name: string;
  way: Way;
  tags?: React.ReactNode;
  subtitle: React.ReactNode;
  right?: React.ReactNode;
  detail?: React.ReactNode; // Noninteractive fact inside an expandable header.
  open?: boolean;
  onToggle?: () => void;
  children?: React.ReactNode;
}> = ({ testId, name, way, tags, subtitle, right, detail, open = false, onToggle, children }) => {
  const avatar = {
    account: 'bg-slate-700 dark:bg-slate-600', subscription: 'bg-emerald-700 dark:bg-emerald-800',
    key: 'bg-blue-700 dark:bg-blue-800', local: 'bg-slate-600 dark:bg-slate-700',
  }[way];
  const head = (
    <>
      <ChevronRight aria-hidden size={12} className={`shrink-0 text-slate-400 transition-transform ${open ? 'rotate-90' : ''} ${onToggle ? '' : 'invisible'}`} />
      <span aria-hidden className={`flex h-[30px] w-[30px] shrink-0 items-center justify-center rounded-full text-[13px] font-bold text-white ${avatar}`}>{name.slice(0, 1).toUpperCase()}</span>
      <span className="min-w-0 flex-1">
        <span className="flex flex-wrap items-center gap-1.5 text-[13px] font-semibold text-slate-700 dark:text-slate-200"><span>{name}</span><ConnectionTag way={way} />{tags}</span>
        <span className={`mt-0.5 block ${hintCls}`}><span className="block truncate">{subtitle}</span></span>
      </span>
      {detail && <span className={`col-start-3 min-w-0 md:col-start-4 md:text-right ${hintCls}`}>{detail}</span>}
    </>
  );
  return (
    <div data-testid="provider-row" data-name={testId} className={`rounded-[10px] border ${open ? 'border-slate-300 bg-white shadow-sm dark:border-white/15 dark:bg-white/[0.03]' : 'border-slate-100 bg-slate-50 dark:border-white/5 dark:bg-white/[0.02]'}`}>
      <div className="flex flex-col items-stretch gap-x-3 gap-y-1.5 px-3 py-2 md:flex-row md:items-center">
        {onToggle
          ? <button type="button" onClick={onToggle} aria-expanded={open} className="grid min-w-0 flex-1 grid-cols-[12px_30px_minmax(0,1fr)] items-center gap-x-3 gap-y-1.5 rounded text-left focus-visible:outline-blue-500 md:grid-cols-[12px_30px_minmax(0,1fr)_auto]">{head}</button>
          : <div className="flex min-w-0 flex-1 items-center gap-3">{head}</div>}
        {right && <div className={`flex min-w-0 max-w-full flex-wrap items-center gap-2 pl-[66px] md:max-w-[42%] md:justify-end md:pl-0 md:text-right ${hintCls}`}>{right}</div>}
      </div>
      {open && children && <div className="px-3 pb-1.5 md:pl-[66px]">{children}</div>}
    </div>
  );
};

/** A ⋯ button and the few things it offers. */
export const RowMenu: React.FC<{ items: { label: string; run: () => void }[]; disabled?: boolean }> = ({ items, disabled }) => {
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const away = (e: MouseEvent) => { if (!box.current?.contains(e.target as Node)) setOpen(false); };
    document.addEventListener('mousedown', away);
    return () => document.removeEventListener('mousedown', away);
  }, [open]);
  return (
    <div ref={box} className="relative shrink-0">
      <button type="button" aria-label={W.options} aria-haspopup="menu" aria-expanded={open} disabled={disabled} onClick={() => setOpen(!open)} className="rounded p-1 text-slate-500 hover:bg-slate-100 dark:hover:bg-white/10 disabled:opacity-50">
        <MoreHorizontal size={16} />
      </button>
      {open && (
        <div role="menu" className="absolute right-0 top-7 z-10 min-w-40 rounded-lg border border-slate-200 bg-white p-1 shadow-lg dark:border-white/10 dark:bg-[#1c1c1c]">
          {items.map((item) => (
            <button key={item.label} type="button" role="menuitem" onClick={() => { setOpen(false); item.run(); }} className="block w-full rounded px-2.5 py-1.5 text-left text-xs hover:bg-slate-100 dark:hover:bg-white/10">
              {item.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
};

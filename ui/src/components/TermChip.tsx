import React from 'react';
import { language } from '../lib/i18n';
import { backdropLabel } from '../lib/skyArt.mts';
import { useSkyStore } from '../stores/skyStore';

/** The 节气 (or festival) whose picture lies behind the window — 今日立冬 on its day, the next term in the title. */
export const TermChip: React.FC = () => {
  const today = useSkyStore((s) => s.today);
  const label = backdropLabel(today, language());
  if (!label) return null;
  const tone = label.today
    ? 'bg-amber-100 dark:bg-amber-500/15 text-amber-700 dark:text-amber-300'
    : 'bg-slate-200/70 dark:bg-white/10 text-slate-500 dark:text-slate-400';
  return (
    <span data-no-i18n title={label.title} className={`hidden md:inline-block whitespace-nowrap shrink-0 text-[10px] px-1.5 py-0.5 rounded font-medium ${tone}`}>
      {label.name}
    </span>
  );
};

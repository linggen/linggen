import React, { useState } from 'react';
import { ChevronRight, ChevronDown } from 'lucide-react';

export const CollapsibleCard: React.FC<{
  title: string;
  icon: React.ReactNode;
  iconColor: string;
  badge?: string;
  defaultOpen?: boolean;
  headerAction?: React.ReactNode;
  children: React.ReactNode;
}> = ({ title, icon, iconColor, badge, defaultOpen = true, headerAction, children }) => {
  const [open, setOpen] = useState(defaultOpen);

  return (
    <section className="bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm flex flex-col overflow-hidden">
      <div className="flex items-center border-b border-slate-200 dark:border-white/5 bg-slate-50 dark:bg-white/[0.02]">
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="flex-1 px-4 py-2 flex items-center justify-between cursor-pointer hover:bg-slate-100 dark:hover:bg-white/[0.04] transition-colors select-none"
        >
          <h3 className={`text-[11px] font-bold uppercase tracking-widest ${iconColor} flex items-center gap-2`}>
            {open ? <ChevronDown size={10} /> : <ChevronRight size={10} />}
            {icon}
            {title}
          </h3>
          {badge && (
            <span className="text-[11px] text-slate-400">{badge}</span>
          )}
        </button>
        {headerAction && (
          <div className="pr-2 flex items-center">{headerAction}</div>
        )}
      </div>
      {open && children}
    </section>
  );
};

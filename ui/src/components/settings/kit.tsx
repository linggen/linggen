/**
 * Shared pieces for the Settings forms: one look for a section, a field, its
 * hint and the details behind it. A hint is one short line (≤20 words); the
 * mechanics go behind "Details".
 */
import React from 'react';
import { ChevronRight } from 'lucide-react';

export const inputCls = 'w-full bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded-lg px-3 py-2 text-xs outline-none focus:ring-1 focus:ring-blue-500/50';
export const labelCls = 'text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400';
export const sectionCls = 'bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm p-5';

export const Section: React.FC<{
  title: string;
  desc?: React.ReactNode;
  className?: string;
  children: React.ReactNode;
}> = ({ title, desc, className, children }) => (
  <section className={className ?? sectionCls}>
    <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300">{title}</h2>
    {desc && <p className="text-[11px] text-slate-500 dark:text-slate-400 mt-1">{desc}</p>}
    <div className="mt-4">{children}</div>
  </section>
);

/** Two columns on a wide screen, one on a phone. */
export const Grid: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">{children}</div>
);

export const Field: React.FC<{
  label?: string;
  hint?: React.ReactNode;
  details?: React.ReactNode;
  wide?: boolean;
  children: React.ReactNode;
}> = ({ label, hint, details, wide, children }) => (
  <div className={wide ? 'sm:col-span-2' : undefined}>
    {label && <label className={labelCls}>{label}</label>}
    <div className={label ? 'mt-1' : undefined}>{children}</div>
    {hint && <p className="text-[11px] text-slate-400 mt-1">{hint}</p>}
    {details && (
      <details className="group mt-0.5">
        <summary className="inline-flex items-center gap-0.5 text-[11px] text-slate-400 hover:text-slate-600 dark:hover:text-slate-300 cursor-pointer select-none list-none [&::-webkit-details-marker]:hidden">
          <ChevronRight size={11} className="transition-transform group-open:rotate-90" /> Details
        </summary>
        <div className="text-[11px] text-slate-400 mt-1 pl-3.5 leading-relaxed">{details}</div>
      </details>
    )}
  </div>
);

/** A collapsed group for the knobs rarely touched. */
export const MoreGroup: React.FC<{ label: string; children: React.ReactNode }> = ({ label, children }) => (
  <details className="group mt-5 border-t border-slate-100 dark:border-white/5 pt-4">
    <summary className="inline-flex items-center gap-1 text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400 hover:text-slate-700 dark:hover:text-slate-200 cursor-pointer select-none list-none [&::-webkit-details-marker]:hidden">
      <ChevronRight size={12} className="transition-transform group-open:rotate-90" /> {label}
    </summary>
    <div className="mt-4">{children}</div>
  </details>
);

/** A number input that commits only values inside [min, max]. */
export const NumberInput: React.FC<{
  value: number;
  min: number;
  max?: number;
  step?: number;
  integer?: boolean;
  placeholder?: string;
  onCommit: (v: number) => void;
}> = ({ value, min, max, step, integer, placeholder, onCommit }) => (
  <input
    className={inputCls}
    type="number"
    min={min}
    max={max}
    step={step ?? 1}
    value={value}
    placeholder={placeholder}
    onChange={(e) => {
      const v = integer ? parseInt(e.target.value, 10) : parseFloat(e.target.value);
      if (Number.isFinite(v) && v >= min && (max === undefined || v <= max)) onCommit(v);
    }}
  />
);

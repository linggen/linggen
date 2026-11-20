/**
 * The one folded mark a notice of Ling's work leaves: Yinyue said it aloud
 * while the person was away (`lib/toldAloud.mts`). It sits on the thing the
 * notice was about — a run's reply, a waiting card — never as a row of its
 * own; opened, it shows what she said.
 */
import React from 'react';
import { toldLabel, type ToldAloud } from '../../lib/toldAloud.mts';

export const ToldAloudMark: React.FC<{ told?: ToldAloud }> = ({ told }) => {
  if (!told) return null;
  return (
    <details data-testid="told-aloud" className="mt-1 text-[11px] text-slate-400 dark:text-slate-500 select-none">
      <summary className="cursor-pointer list-none hover:text-slate-600 dark:hover:text-slate-300">{toldLabel(told)}</summary>
      {told.line && <div className="mt-0.5 pl-3 italic text-slate-500 dark:text-slate-400 select-text">{told.line}</div>}
    </details>
  );
};

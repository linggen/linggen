// The line under the chat: "✶ Musing… (12s · 30 tok/s · 4.2k/200K ctx)"
// while a run is going, "✻ Musing for 12s" after it.
import React from 'react';
import type { RunSummary } from './useRunSpinner';

const duration = (s: number) => (s >= 60 ? `${Math.floor(s / 60)}m ${s % 60}s` : `${s}s`);

function contextLabel(tokens: number, limit?: number): string {
  const tk = `${(tokens / 1000).toFixed(1)}k`;
  if (!limit) return `${tk} ctx`;
  const lim = limit >= 1_000_000
    ? `${(limit / 1_000_000).toFixed(limit % 1_000_000 === 0 ? 0 : 1)}M`
    : `${Math.round(limit / 1000)}K`;
  return `${tk}/${lim} ctx (${Math.round((tokens / limit) * 100)}%)`;
}

export const RunStatusLine: React.FC<{
  active: boolean;
  reconnecting: boolean;
  label: string;
  elapsed: number;
  tokensPerSec?: number;
  context?: { tokens: number; tokenLimit?: number };
  summary: RunSummary | null;
}> = ({ active, reconnecting, label, elapsed, tokensPerSec, context, summary }) => {
  // While the link is down nothing about the run is known — it may have died
  // with the engine — so the line says what is actually true.
  if (active && reconnecting) {
    return (
      <div className="px-3 py-1.5">
        <div className="flex items-center gap-1.5 text-[13px] text-amber-600 dark:text-amber-400 font-medium">
          <span>⚠</span>
          <span>Lost the connection to Linggen — reconnecting. This turn may have stopped.</span>
        </div>
      </div>
    );
  }
  if (active) {
    const tokens = context?.tokens ?? 0;
    const details = [
      elapsed > 0 ? duration(elapsed) : '',
      (tokensPerSec ?? 0) > 0 ? `${tokensPerSec!.toFixed(1)} tok/s` : '',
      tokens > 0 ? contextLabel(tokens, context?.tokenLimit) : '',
    ].filter(Boolean);
    return (
      <div className="px-3 py-1.5">
        <div className="flex items-center gap-1.5 text-[13px] text-slate-500 dark:text-slate-400 font-medium animate-pulse">
          <span className="text-blue-500">✶</span>
          <span>
            {label}…
            {(elapsed > 0 || tokens > 0) && (
              <span className="font-normal text-slate-400 dark:text-slate-500 ml-1">({details.join(' · ')})</span>
            )}
          </span>
        </div>
      </div>
    );
  }
  if (!summary) return null;
  return (
    <div className="px-3 py-1.5">
      <div className={`flex items-center gap-1.5 text-[13px] italic ${summary.interrupted ? 'text-red-400 dark:text-red-400/80' : 'text-slate-400 dark:text-slate-500'}`}>
        <span>✻</span>
        <span>
          {summary.interrupted ? 'Interrupted after' : `${summary.verb} for`} {duration(summary.elapsed)}
        </span>
      </div>
    </div>
  );
};

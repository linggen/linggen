import React, { useEffect, useState } from 'react';
import { memory, type MemoryStats as Stats } from './memoryApi.mts';
import { fmtBytes, relAge } from './memoryFormat.mts';

/** Rows per tier, the store's size and the last dream — one quiet line.
 *  Hidden when `stats` is not available to this viewer. `version` changes
 *  when rows were added or removed, so the numbers follow. */
export const MemoryStats: React.FC<{ version: number }> = ({ version }) => {
  const [stats, setStats] = useState<Stats | null>(null);

  useEffect(() => {
    let live = true;
    memory.stats().then((s) => live && setStats(s), () => live && setStats(null));
    return () => { live = false; };
  }, [version]);

  if (!stats) return null;
  const dream = stats.last_remembered_at ? relAge(stats.last_remembered_at) : 'never';
  return (
    <div data-testid="memory-stats" className="flex flex-wrap gap-x-4 gap-y-1 text-[11px] text-slate-500 dark:text-slate-400">
      <span><b className="text-slate-700 dark:text-slate-200">{stats.total}</b> {stats.total === 1 ? 'row' : 'rows'}</span>
      <span>Core {stats.per_tier.core}</span>
      <span>Long-term {stats.per_tier.semantic}</span>
      <span>Episodic {stats.per_tier.episodic}</span>
      {stats.expired > 0 && <span>Archived {stats.expired}</span>}
      <span>{fmtBytes(stats.disk_bytes.total)} on disk</span>
      <span>Last dream: {dream}</span>
    </div>
  );
};

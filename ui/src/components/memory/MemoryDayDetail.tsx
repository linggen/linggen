import React from 'react';
import { dayState } from './memoryCalendar.mts';
import { fmtTime } from './memoryFormat.mts';
import type { MemoryDay } from './memoryApi.mts';
import { ghostBtn, sectionCls, tierChipCls, chipCls } from './memoryStyle';

const labels = { today: 'Today', dreamed: 'Dreamed', scanned: 'Scanned', waiting: 'Waiting for a dream', empty: 'Empty' };
const tiers = { today: 'semantic', dreamed: 'episodic', scanned: 'semantic', waiting: 'core', empty: '' };

export function MemoryDayDetail({ date, day, today, busy, start }: {
  date: string; day?: MemoryDay; today: string; busy: boolean; start: (date: string) => void;
}) {
  const state = dayState(day ?? { date, rows: 0, unjudged: 0 }, today);
  const dreamed = day?.dreamed || day?.state === 'dreamed';
  return <section className={`${sectionCls} space-y-2`}>
    <div className="flex flex-wrap items-center gap-2">
      <strong className="text-xs text-slate-700 dark:text-slate-300">{date}</strong>
      <span className={state === 'empty' ? chipCls : tierChipCls(tiers[state])}>{labels[state]}</span>
    </div>
    <div className="flex flex-wrap gap-x-3 gap-y-1 text-xs text-slate-500">
      <span>{`${day?.rows ?? 0} rows`}</span><span>{`${day?.unjudged ?? 0} unjudged`}</span>
      {day?.promoted != null && <span>{`${day.promoted} promoted`}</span>}
      {day?.forgotten != null && <span>{`${day.forgotten} forgotten`}</span>}
    </div>
    {day?.harvested_at && <p className="text-xs text-slate-500">{`Scanned at: ${fmtTime(day.harvested_at)}`}</p>}
    {day?.remembered_at && <p className="text-xs text-slate-500">{`Dreamed at: ${fmtTime(day.remembered_at)}`}</p>}
    {date === today && <p className="text-xs text-slate-500">Today's rows are staged; they can be dreamed after midnight.</p>}
    {date < today && <div className="flex flex-wrap gap-2">
      {!day?.scanned && <button className={ghostBtn} disabled title="Coming with the next update.">Scan this day</button>}
      {!dreamed && <button className={ghostBtn} disabled={busy} onClick={() => start(date)}>Dream this day</button>}
    </div>}
  </section>;
}

import React from 'react';
import { dayState, monthGrid, type DayState } from './memoryCalendar.mts';
import type { MemoryDay } from './memoryApi.mts';

// These are the foreground colours of the Memory tab's existing tier chips.
const marks: Record<DayState, string> = {
  today: 'bg-blue-700 dark:bg-blue-400', dreamed: 'bg-violet-700 dark:bg-violet-400',
  scanned: 'bg-blue-700 dark:bg-blue-400', waiting: 'bg-amber-700 dark:bg-amber-400',
  empty: 'bg-slate-500 dark:bg-slate-400',
};
const weekdays = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];

export function MemoryCalendarGrid({ year, month, days, today, selected, select }: {
  year: number; month: number; days: MemoryDay[]; today: string; selected: string | null; select: (date: string) => void;
}) {
  const byDate = new Map(days.map((day) => [day.date, day]));
  return <div className="grid grid-cols-7 gap-1">
    {weekdays.map((name) => <div key={name} className="text-center text-xs font-semibold text-slate-500">{name}</div>)}
    {monthGrid(year, month).flat().map((date, index) => {
      if (!date) return <div key={`blank-${index}`} />;
      const day = byDate.get(date);
      const state = dayState(day ?? { date, rows: 0, unjudged: 0 }, today);
      return <button key={date} data-testid={day ? 'memory-day' : undefined} aria-label={date}
        aria-pressed={selected === date} onClick={() => select(date)}
        className={`min-h-14 rounded-lg p-1 text-xs text-slate-700 hover:bg-slate-100 dark:text-slate-300 dark:hover:bg-white/5 ${selected === date ? 'ring-2 ring-blue-600' : ''}`}>
        <span className="flex items-center justify-center gap-1">
          {Number(date.slice(8))}<span className={`h-1.5 w-1.5 rounded-full ${marks[state]}`} />
        </span>
        {day && day.rows > 0 && <span className="block text-[10px] text-slate-500">{`${day.rows} rows`}</span>}
      </button>;
    })}
  </div>;
}

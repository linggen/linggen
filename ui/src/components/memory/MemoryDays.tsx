import React, { useCallback, useEffect, useRef, useState } from 'react';
import { startDream } from './dreamApi.mts';
import { describeError, memory, MemoryError, type MemoryDays as Days } from './memoryApi.mts';
import { ghostBtn, primaryBtn } from './memoryStyle';
import { monthOf, shiftMonth } from './memoryCalendar.mts';
import { MemoryCalendarGrid } from './MemoryCalendarGrid';
import { MemoryDayDetail } from './MemoryDayDetail';

export const MemoryDays: React.FC = () => {
  const [data, setData] = useState<Days | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [notice, setNotice] = useState('');
  const [busy, setBusy] = useState(false);
  const [month, setMonth] = useState<{ year: number; month: number } | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const mounted = useRef(false);
  const timer = useRef<ReturnType<typeof setInterval> | null>(null);
  const stop = useCallback(() => { if (timer.current) clearInterval(timer.current); timer.current = null; }, []);
  const load = useCallback(async () => {
    try { setData(await memory.days()); setError(null); } catch (e) { setError(e); }
  }, []);
  useEffect(() => { mounted.current = true; void load(); return () => { mounted.current = false; stop(); }; }, [load, stop]);
  const start = async (day?: string) => {
    setBusy(true); setNotice('');
    try {
      await startDream(day); if (!mounted.current) return; setNotice('Dream started.'); void load(); stop();
      const until = Date.now() + 180_000;
      timer.current = setInterval(() => { if (Date.now() >= until) { stop(); return; } void load(); }, 5000);
    } catch (e) { setNotice(e instanceof Error ? e.message : 'Could not start dream.'); }
    finally { setBusy(false); }
  };
  const days = data?.days ?? [];
  const waiting = days.filter((d) => d.date < (data?.today ?? '') && d.unjudged > 0).length;
  const shown = month ?? (data ? monthOf(data.today) : null);
  const current = data ? monthOf(data.today) : null;
  const atCurrent = shown && current && shown.year * 12 + shown.month >= current.year * 12 + current.month;
  const move = (delta: number) => { if (shown) setMonth(shiftMonth(shown.year, shown.month, delta)); setSelected(null); };
  return (
    <div className="space-y-3" data-testid="memory-subview-days">
      <div className="flex flex-wrap items-center gap-2">
        {shown && <>
          <button className={ghostBtn} aria-label="Previous month" onClick={() => move(-1)}>‹</button>
          <h3 className="text-sm font-bold text-slate-700 dark:text-slate-300">{new Date(shown.year, shown.month - 1, 1).toLocaleDateString(undefined, { month: 'long', year: 'numeric' })}</h3>
          <button className={ghostBtn} aria-label="Next month" disabled={Boolean(atCurrent)} onClick={() => move(1)}>›</button>
        </>}
        <button className={primaryBtn} disabled={busy} onClick={() => start()}>Dream now</button>
      </div>
      {notice && <p role="status" className="text-xs text-slate-500">{notice}</p>}
      {error != null && <div role="alert" className="text-xs text-red-600 dark:text-red-400">{describeError(error, 'read memory')}{!(error instanceof MemoryError && error.forbidden) && <button className={ghostBtn} onClick={load}>Try again</button>}</div>}
      {!data && !error && <p className="text-xs text-slate-500">Loading…</p>}
      {data && <p className="text-xs text-slate-500">{`${waiting} days wait for a dream.`}</p>}
      {data && days.length === 0 && <p className="text-xs text-slate-500">No days yet.</p>}
      {data && shown && <MemoryCalendarGrid {...shown} days={days} today={data.today} selected={selected} select={setSelected} />}
      {data && selected && <MemoryDayDetail date={selected} day={days.find((day) => day.date === selected)} today={data.today} busy={busy} start={start} />}
    </div>
  );
};

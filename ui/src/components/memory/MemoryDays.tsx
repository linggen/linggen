import React, { useCallback, useEffect, useRef, useState } from 'react';
import { startDream, startScan } from './dreamApi.mts';
import { OpenRun } from './OpenRun';
import { describeError, memory, MemoryError, type MemoryDays as Days } from './memoryApi.mts';
import { ghostBtn, primaryBtn } from './memoryStyle';
import { monthOf, shiftMonth } from './memoryCalendar.mts';
import { uiLocale } from './memoryFormat.mts';
import { MemoryCalendarGrid } from './MemoryCalendarGrid';
import { MemoryDayDetail } from './MemoryDayDetail';

export const MemoryDays: React.FC<{ canOpenRun: boolean }> = ({ canOpenRun }) => {
  const [data, setData] = useState<Days | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [notice, setNotice] = useState('');
  const [run, setRun] = useState<string | null>(null);
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
  const launch = async (request: () => Promise<string | null>, success: string, failure: string) => {
    setBusy(true); setNotice(''); setRun(null);
    try {
      const session = await request(); if (!mounted.current) return; setRun(session); setNotice(success); void load(); stop();
      const until = Date.now() + 180_000;
      timer.current = setInterval(() => { if (Date.now() >= until) { stop(); return; } void load(); }, 5000);
    } catch (e) { if (mounted.current) setNotice(e instanceof Error ? e.message : failure); }
    finally { if (mounted.current) setBusy(false); }
  };
  const start = (day?: string) => launch(() => startDream(day), 'Dream started.', 'Could not start dream.');
  const scan = (day: string) => launch(() => startScan(day), 'Scan started.', 'Could not start the scan.');
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
          <button className={`${ghostBtn} min-w-11 justify-center md:min-w-0`} aria-label="Previous month" onClick={() => move(-1)}>‹</button>
          <h3 className="text-sm font-bold text-slate-700 dark:text-slate-300">{new Date(shown.year, shown.month - 1, 1).toLocaleDateString(uiLocale(), { month: 'long', year: 'numeric' })}</h3>
          <button className={`${ghostBtn} min-w-11 justify-center md:min-w-0`} aria-label="Next month" disabled={Boolean(atCurrent)} onClick={() => move(1)}>›</button>
        </>}
        <button className={primaryBtn} disabled={busy} onClick={() => start()}>Dream now</button>
      </div>
      {notice && <div role="status" className="flex items-center gap-2 text-xs text-slate-500"><span>{notice}</span>{canOpenRun && run && <OpenRun sessionId={run} />}</div>}
      {error != null && <div role="alert" className="text-xs text-red-600 dark:text-red-400">{describeError(error, 'read memory')}{!(error instanceof MemoryError && error.forbidden) && <button className={ghostBtn} onClick={load}>Try again</button>}</div>}
      {!data && !error && <p className="text-xs text-slate-500">Loading…</p>}
      {data && <p className="text-xs text-slate-500">{`${waiting} days wait for a dream.`}</p>}
      {data && days.length === 0 && <p className="text-xs text-slate-500">No days yet.</p>}
      {data && shown && <MemoryCalendarGrid {...shown} days={days} today={data.today} selected={selected} select={setSelected} />}
      {data && selected && <MemoryDayDetail date={selected} day={days.find((day) => day.date === selected)} today={data.today} busy={busy} start={start} scan={scan} />}
    </div>
  );
};
